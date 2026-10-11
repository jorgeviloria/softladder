//! Modbus master driver — **M5 slice four: the poll loop as an [`IoDriver`](crate::IoDriver)**.
//!
//! [`ModbusMasterDriver`] polls a Modbus TCP slave through a [`ModbusMap`](crate::ModbusMap):
//! [`IoDriver::read`](crate::IoDriver::read) fetches discrete inputs and input registers into
//! the image, [`IoDriver::write`](crate::IoDriver::write) pushes coils and holding registers
//! out of it. Multi-register runs are chunked to the protocol limits (2000 bits, 125 registers
//! per read; 1968 coils, 123 registers per write), and anything the map does not cover is
//! skipped, following the overlap rule of [`SimDriver`](crate::SimDriver).
//!
//! Any failure — socket, timeout, protocol or slave exception — surfaces as
//! [`IoError::Driver`](crate::IoError::Driver). The driver owns the retry policy (via
//! [`TcpMaster`](crate::modbus_tcp::TcpMaster)); the fail-safe output policy stays with the
//! runtime, as `docs/ARCHITECTURE.md` §8 requires.

use crate::image::{IoDriver, IoError, IoImage};
use crate::modbus::ModbusMap;
use crate::modbus_tcp::TcpMaster;

/// Polling Modbus TCP master exposing the mapped channels as an [`IoDriver`].
#[derive(Debug)]
pub struct ModbusMasterDriver {
    master: TcpMaster,
    map: ModbusMap,
}

impl ModbusMasterDriver {
    /// Creates a driver polling through `master` over `map`.
    pub fn new(master: TcpMaster, map: ModbusMap) -> Self {
        Self { master, map }
    }

    /// Underlying master (address, unit, timeout, retries).
    pub fn master(&self) -> &TcpMaster {
        &self.master
    }

    /// Mutable access to the underlying master.
    pub fn master_mut(&mut self) -> &mut TcpMaster {
        &mut self.master
    }

    /// Configured map.
    pub fn map(&self) -> &ModbusMap {
        &self.map
    }

    fn read_bits(&mut self, base: u16, total: usize, discrete: bool) -> Result<Vec<bool>, IoError> {
        let mut out = Vec::with_capacity(total);
        let mut done = 0usize;
        while done < total {
            let len = (total - done).min(2000);
            let addr = offset(base, done)?;
            let count = len_as_u16(len)?;
            let mut chunk = if discrete {
                self.master.read_discrete(addr, count)
            } else {
                self.master.read_coils(addr, count)
            }
            .map_err(driver_error)?;
            out.append(&mut chunk);
            done += len;
        }
        Ok(out)
    }

    fn read_regs(&mut self, base: u16, total: usize, input: bool) -> Result<Vec<u16>, IoError> {
        let mut out = Vec::with_capacity(total);
        let mut done = 0usize;
        while done < total {
            let len = (total - done).min(125);
            let addr = offset(base, done)?;
            let count = len_as_u16(len)?;
            let mut chunk = if input {
                self.master.read_input(addr, count)
            } else {
                self.master.read_holding(addr, count)
            }
            .map_err(driver_error)?;
            out.append(&mut chunk);
            done += len;
        }
        Ok(out)
    }

    fn write_bits(&mut self, base: u16, values: &[bool]) -> Result<(), IoError> {
        for (index, chunk) in values.chunks(1968).enumerate() {
            let addr = offset(base, index.saturating_mul(1968))?;
            self.master.write_coils(addr, chunk).map_err(driver_error)?;
        }
        Ok(())
    }

    fn write_regs(&mut self, base: u16, values: &[u16]) -> Result<(), IoError> {
        for (index, chunk) in values.chunks(123).enumerate() {
            let addr = offset(base, index.saturating_mul(123))?;
            self.master
                .write_registers(addr, chunk)
                .map_err(driver_error)?;
        }
        Ok(())
    }
}

impl IoDriver for ModbusMasterDriver {
    fn name(&self) -> &str {
        "modbus-tcp-master"
    }

    fn read(&mut self, image: &mut IoImage) -> Result<(), IoError> {
        let digital = self.map.digital_count.min(image.digital_in.len());
        if digital > 0 {
            let values = self.read_bits(self.map.discrete_base, digital, true)?;
            image.digital_in[..digital].copy_from_slice(&values);
        }
        let analog = self.map.analog_in;
        if analog.count > 0 && analog.register_len() > 0 && !image.analog_in.is_empty() {
            let registers = self.read_regs(analog.register_base, analog.register_len(), true)?;
            self.map.decode_registers(&registers, &mut image.analog_in);
        }
        Ok(())
    }

    fn write(&mut self, image: &IoImage) -> Result<(), IoError> {
        let digital = self.map.digital_count.min(image.digital_out.len());
        if digital > 0 {
            let coils = self.map.encode_coils(&image.digital_out);
            self.write_bits(self.map.coil_base, &coils[..digital])?;
        }
        let analog = self.map.analog_out;
        if analog.count > 0 && analog.register_len() > 0 && !image.analog_out.is_empty() {
            let registers = self.map.encode_registers(&image.analog_out);
            self.write_regs(analog.register_base, &registers)?;
        }
        Ok(())
    }
}

fn offset(base: u16, done: usize) -> Result<u16, IoError> {
    u16::try_from(done)
        .ok()
        .and_then(|delta| base.checked_add(delta))
        .ok_or_else(|| driver_error_string(format!("address past 0xFFFF from base {base}")))
}

fn len_as_u16(len: usize) -> Result<u16, IoError> {
    u16::try_from(len).map_err(|_| driver_error_string(format!("count {len} past 0xFFFF")))
}

fn driver_error(error: crate::modbus_tcp::TcpError) -> IoError {
    driver_error_string(error.to_string())
}

fn driver_error_string(message: String) -> IoError {
    IoError::Driver {
        driver: "modbus-tcp-master".to_owned(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Duration;

    use crate::modbus_tcp::{serve_stream, SlaveStore};

    fn loopback_pair(
        map: ModbusMap,
        connections: usize,
    ) -> (ModbusMasterDriver, std::thread::JoinHandle<SlaveStore>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let store_map = map.clone();
        let server = std::thread::spawn(move || {
            let mut store = SlaveStore::new(store_map);
            store.set_discrete(0, true);
            store.set_input_register(0, 0x00AB);
            for _ in 0..connections {
                let (stream, _) = listener.accept().expect("master connects");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout sets");
                serve_stream(stream, &mut store, 1).expect("slave serves");
            }
            store
        });
        let master = TcpMaster::new(addr, 1, Duration::from_secs(5), 0);
        (ModbusMasterDriver::new(master, map), server)
    }

    #[test]
    fn name_identifies_the_driver() {
        let map = ModbusMap::fixed(2, 1);
        let master = TcpMaster::new(
            "127.0.0.1:502".parse().expect("address parses"),
            1,
            Duration::from_secs(1),
            0,
        );
        let driver = ModbusMasterDriver::new(master, map);
        assert_eq!(driver.name(), "modbus-tcp-master");
        assert_eq!(driver.map().digital_count, 2);
    }

    #[test]
    fn read_and_write_round_trip_through_a_loopback_slave() {
        let map = ModbusMap::fixed(8, 2);
        // One connection per master call: read polls discrete + input (2),
        // write pushes coils + holding (2).
        let (mut driver, server) = loopback_pair(map, 4);

        let mut image = IoImage::new(8, 2);
        driver.read(&mut image).expect("poll succeeds");
        assert!(image.digital_input(0).unwrap_or(false));
        assert!(!image.digital_input(1).unwrap_or(true));
        assert_eq!(image.analog_input(0), Some(171.0));

        image.set_digital_output(3, true).expect("in range");
        image.set_analog_output(1, 9.0).expect("in range");
        driver.write(&image).expect("push succeeds");

        let store = server.join().expect("server joins");
        assert!(store.coils()[3]);
        assert!(!store.coils()[0]);
        assert_eq!(store.holding()[1], 9);
    }

    #[test]
    fn long_runs_are_chunked_to_the_protocol_limits() {
        let map = ModbusMap::fixed(3000, 0);
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let server = std::thread::spawn(move || {
            let mut store = SlaveStore::new(ModbusMap::fixed(3000, 0));
            store.set_discrete(2500, true);
            // 3000 coils need two reads (2000 + 1000).
            for _ in 0..2 {
                let (stream, _) = listener.accept().expect("master connects");
                serve_stream(stream, &mut store, 1).expect("slave serves");
            }
        });

        let master = TcpMaster::new(addr, 1, Duration::from_secs(5), 0);
        let mut driver = ModbusMasterDriver::new(master, map);
        let mut image = IoImage::new(3000, 0);
        driver.read(&mut image).expect("chunked poll succeeds");
        assert!(!image.digital_input(0).unwrap_or(true));
        assert!(image.digital_input(2500).unwrap_or(false));
        server.join().expect("server joins");
    }

    #[test]
    fn a_dead_slave_surfaces_a_driver_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        drop(listener);
        let master = TcpMaster::new(addr, 1, Duration::from_millis(100), 0);
        let mut driver = ModbusMasterDriver::new(master, ModbusMap::fixed(4, 1));
        let mut image = IoImage::new(4, 1);
        match driver.read(&mut image) {
            Err(IoError::Driver { driver, .. }) => assert_eq!(driver, "modbus-tcp-master"),
            other => panic!("expected a driver error, got {other:?}"),
        }
    }
}
