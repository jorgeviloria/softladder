//! Modbus TCP transport — **M5 slice two (blocking, `std` only)**.
//!
//! Pure MBAP/PDU codec plus a blocking master with timeout and retries and
//! an in-memory slave that exposes the wire side of [`ModbusMap`](crate::ModbusMap).
//! Function codes `01`/`02`/`03`/`04`/`05`/`06`/`0F`/`10` and exception frames
//! are covered — the ClassicLadder slave set. No async runtime and no third-party
//! crate: the transport is `std::net::TcpStream` with read/write timeouts, so the
//! MSRV stays where the workspace pins it.
//!
//! The slave validates addresses against the configured map: a request outside
//! the exposed runs answers `IllegalDataAddress` (`0x02`) instead of panicking.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use thiserror::Error;

/// Default Modbus TCP port.
pub const DEFAULT_PORT: u16 = 502;

/// Failure of a Modbus TCP exchange (also reused by the RTU framing codec,
/// which shares the PDU and the exception model and only differs in framing).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TcpError {
    /// The socket failed.
    #[error("modbus transport failed: {0}")]
    Transport(String),
    /// A well-formed frame was unexpected (bad echo, truncation, bad length).
    #[error("modbus protocol error: {0}")]
    Protocol(String),
    /// The server answered with an exception frame.
    #[error("modbus exception: function 0x{function:02X}, code 0x{code:02X}")]
    Exception {
        /// Echoed function without the exception bit.
        function: u8,
        /// Exception code (`0x01`–`0x04`).
        code: u8,
    },
}

/// Exception codes a slave may answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionCode {
    /// `0x01`: the function is not supported.
    IllegalFunction = 0x01,
    /// `0x02`: no mapped address covers the request.
    IllegalDataAddress = 0x02,
    /// `0x03`: the value or count field is not allowed.
    IllegalDataValue = 0x03,
    /// `0x04`: the slave failed to handle the request.
    ServerFailure = 0x04,
}

/// A Modbus request PDU (function + address + payload).
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// `0x01`: read `count` coils from `addr`.
    ReadCoils {
        /// First coil address.
        addr: u16,
        /// Coils to read.
        count: u16,
    },
    /// `0x02`: read `count` discrete inputs from `addr`.
    ReadDiscrete {
        /// First input address.
        addr: u16,
        /// Inputs to read.
        count: u16,
    },
    /// `0x03`: read `count` holding registers from `addr`.
    ReadHolding {
        /// First register address.
        addr: u16,
        /// Registers to read.
        count: u16,
    },
    /// `0x04`: read `count` input registers from `addr`.
    ReadInput {
        /// First register address.
        addr: u16,
        /// Registers to read.
        count: u16,
    },
    /// `0x05`: write one coil.
    WriteCoil {
        /// Coil address.
        addr: u16,
        /// State to write.
        value: bool,
    },
    /// `0x06`: write one holding register.
    WriteRegister {
        /// Register address.
        addr: u16,
        /// Value to write.
        value: u16,
    },
    /// `0x0F`: write several coils from `addr`.
    WriteCoils {
        /// First coil address.
        addr: u16,
        /// States to write.
        values: Vec<bool>,
    },
    /// `0x10`: write several holding registers from `addr`.
    WriteRegisters {
        /// First register address.
        addr: u16,
        /// Values to write.
        values: Vec<u16>,
    },
}

impl Request {
    /// Function code of the request.
    pub fn function(&self) -> u8 {
        match self {
            Request::ReadCoils { .. } => 0x01,
            Request::ReadDiscrete { .. } => 0x02,
            Request::ReadHolding { .. } => 0x03,
            Request::ReadInput { .. } => 0x04,
            Request::WriteCoil { .. } => 0x05,
            Request::WriteRegister { .. } => 0x06,
            Request::WriteCoils { .. } => 0x0F,
            Request::WriteRegisters { .. } => 0x10,
        }
    }
}

/// A Modbus response PDU.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// `0x01` answer: coil states.
    Coils(Vec<bool>),
    /// `0x02` answer: discrete input states.
    Discrete(Vec<bool>),
    /// `0x03` answer: holding registers.
    Holding(Vec<u16>),
    /// `0x04` answer: input registers.
    Input(Vec<u16>),
    /// `0x05`/`0x06` answer: echoed address and value.
    WrittenSingle {
        /// Echoed function (`0x05` or `0x06`).
        function: u8,
        /// Echoed address.
        addr: u16,
        /// Echoed value (coil as `0xFF00`/`0x0000`, or the register).
        value: u16,
    },
    /// `0x0F`/`0x10` answer: echoed address and count.
    WrittenMultiple {
        /// Echoed function (`0x0F` or `0x10`).
        function: u8,
        /// Echoed first address.
        addr: u16,
        /// Echoed item count.
        count: u16,
    },
}

/// Packs coil states LSB-first, as the `0x01`/`0x02`/`0x0F` payloads require.
pub fn pack_coils(values: &[bool]) -> Vec<u8> {
    values
        .chunks(8)
        .map(|chunk| {
            chunk.iter().enumerate().fold(
                0u8,
                |byte, (bit, state)| {
                    if *state {
                        byte | (1 << bit)
                    } else {
                        byte
                    }
                },
            )
        })
        .collect()
}

/// Unpacks `count` LSB-first coil states.
pub fn unpack_coils(data: &[u8], count: usize) -> Vec<bool> {
    (0..count)
        .map(|index| {
            data.get(index / 8)
                .is_some_and(|byte| byte & (1 << (index % 8)) != 0)
        })
        .collect()
}

/// Encodes a full Modbus TCP frame (MBAP header + PDU).
pub fn encode_request(transaction: u16, unit: u8, request: &Request) -> Vec<u8> {
    let mut pdu = vec![request.function()];
    match request {
        Request::ReadCoils { addr, count }
        | Request::ReadDiscrete { addr, count }
        | Request::ReadHolding { addr, count }
        | Request::ReadInput { addr, count } => {
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, *count);
        }
        Request::WriteCoil { addr, value } => {
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, if *value { 0xFF00 } else { 0x0000 });
        }
        Request::WriteRegister { addr, value } => {
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, *value);
        }
        Request::WriteCoils { addr, values } => {
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, values.len() as u16);
            let packed = pack_coils(values);
            pdu.push(packed.len() as u8);
            pdu.extend_from_slice(&packed);
        }
        Request::WriteRegisters { addr, values } => {
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, values.len() as u16);
            pdu.push((values.len() * 2) as u8);
            for value in values {
                push_u16(&mut pdu, *value);
            }
        }
    }
    wrap_frame(transaction, unit, &pdu)
}

/// Decodes a Modbus TCP request frame into transaction, unit and PDU.
///
/// An unknown function decodes as [`TcpError::Exception`] with
/// [`ExceptionCode::IllegalFunction`], so a server can answer it directly.
pub fn decode_request(frame: &[u8]) -> Result<(u16, u8, Request), TcpError> {
    let (transaction, unit, pdu) = split_frame(frame)?;
    let function = *pdu
        .first()
        .ok_or_else(|| TcpError::Protocol("empty PDU".to_owned()))?;
    let body = &pdu[1..];
    let request = match function {
        0x01..=0x04 => {
            let (addr, count) = two_u16(body, function)?;
            match function {
                0x01 => Request::ReadCoils { addr, count },
                0x02 => Request::ReadDiscrete { addr, count },
                0x03 => Request::ReadHolding { addr, count },
                _ => Request::ReadInput { addr, count },
            }
        }
        0x05 => {
            let (addr, raw) = two_u16(body, function)?;
            match raw {
                0xFF00 => Request::WriteCoil { addr, value: true },
                0x0000 => Request::WriteCoil { addr, value: false },
                _ => {
                    return Err(TcpError::Protocol(format!(
                        "function 0x05 with value 0x{raw:04X}"
                    )));
                }
            }
        }
        0x06 => {
            let (addr, value) = two_u16(body, function)?;
            Request::WriteRegister { addr, value }
        }
        0x0F => {
            let (addr, values) = multi_coils(body)?;
            Request::WriteCoils { addr, values }
        }
        0x10 => {
            let (addr, values) = multi_registers(body)?;
            Request::WriteRegisters { addr, values }
        }
        _ => {
            return Err(TcpError::Exception {
                function,
                code: ExceptionCode::IllegalFunction as u8,
            });
        }
    };
    Ok((transaction, unit, request))
}

/// Encodes a response PDU into a full Modbus TCP frame.
pub fn encode_response(transaction: u16, unit: u8, response: &Response) -> Vec<u8> {
    let mut pdu = Vec::new();
    match response {
        Response::Coils(values) => {
            pdu.push(0x01);
            let packed = pack_coils(values);
            pdu.push(packed.len() as u8);
            pdu.extend_from_slice(&packed);
        }
        Response::Discrete(values) => {
            pdu.push(0x02);
            let packed = pack_coils(values);
            pdu.push(packed.len() as u8);
            pdu.extend_from_slice(&packed);
        }
        Response::Holding(values) => {
            pdu.push(0x03);
            pdu.push((values.len() * 2) as u8);
            for value in values {
                push_u16(&mut pdu, *value);
            }
        }
        Response::Input(values) => {
            pdu.push(0x04);
            pdu.push((values.len() * 2) as u8);
            for value in values {
                push_u16(&mut pdu, *value);
            }
        }
        Response::WrittenSingle {
            function,
            addr,
            value,
        } => {
            pdu.push(*function);
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, *value);
        }
        Response::WrittenMultiple {
            function,
            addr,
            count,
        } => {
            pdu.push(*function);
            push_u16(&mut pdu, *addr);
            push_u16(&mut pdu, *count);
        }
    }
    wrap_frame(transaction, unit, &pdu)
}

/// Encodes an exception frame for `function` with `code`.
pub fn encode_exception(transaction: u16, unit: u8, function: u8, code: u8) -> Vec<u8> {
    wrap_frame(transaction, unit, &[function | 0x80, code])
}

/// Decodes a response frame, returning transaction id, unit id and PDU.
///
/// Exception frames become [`TcpError::Exception`]; anything else malformed
/// becomes [`TcpError::Protocol`]. The caller matches the PDU against the
/// request it sent (see [`TcpMaster`]).
pub fn decode_response(frame: &[u8]) -> Result<(u16, u8, Vec<u8>), TcpError> {
    let (transaction, unit, pdu) = split_frame(frame)?;
    if pdu.len() < 2 {
        return Err(TcpError::Protocol("response PDU too short".to_owned()));
    }
    if pdu[0] & 0x80 != 0 {
        return Err(TcpError::Exception {
            function: pdu[0] & 0x7F,
            code: pdu[1],
        });
    }
    Ok((transaction, unit, pdu.to_vec()))
}

/// Blocking Modbus TCP master with timeout and retries.
///
/// Each call opens a fresh connection (the ClassicLadder poll pattern),
/// sends one request and waits for its response. Transport and protocol
/// failures are retried `retries` times; exception answers are returned
/// immediately since retrying them cannot succeed.
#[derive(Debug)]
pub struct TcpMaster {
    addr: SocketAddr,
    unit_id: u8,
    timeout: Duration,
    retries: u32,
    next_transaction: u16,
}

impl TcpMaster {
    /// Creates a master for `addr` speaking to unit `unit_id`.
    ///
    /// `timeout` bounds connect, read and write; `retries` counts the extra
    /// attempts after the first one fails.
    pub fn new(addr: SocketAddr, unit_id: u8, timeout: Duration, retries: u32) -> Self {
        Self {
            addr,
            unit_id,
            timeout,
            retries,
            next_transaction: 0,
        }
    }

    /// Socket address of the slave.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Unit id sent in the MBAP header.
    pub fn unit_id(&self) -> u8 {
        self.unit_id
    }

    /// Reads `count` coils from `addr`.
    pub fn read_coils(&mut self, addr: u16, count: u16) -> Result<Vec<bool>, TcpError> {
        match self.transact(&Request::ReadCoils { addr, count })? {
            Response::Coils(values) => Ok(values),
            other => Err(unexpected("0x01", &other)),
        }
    }

    /// Reads `count` discrete inputs from `addr`.
    pub fn read_discrete(&mut self, addr: u16, count: u16) -> Result<Vec<bool>, TcpError> {
        match self.transact(&Request::ReadDiscrete { addr, count })? {
            Response::Discrete(values) => Ok(values),
            other => Err(unexpected("0x02", &other)),
        }
    }

    /// Reads `count` holding registers from `addr`.
    pub fn read_holding(&mut self, addr: u16, count: u16) -> Result<Vec<u16>, TcpError> {
        match self.transact(&Request::ReadHolding { addr, count })? {
            Response::Holding(values) => Ok(values),
            other => Err(unexpected("0x03", &other)),
        }
    }

    /// Reads `count` input registers from `addr`.
    pub fn read_input(&mut self, addr: u16, count: u16) -> Result<Vec<u16>, TcpError> {
        match self.transact(&Request::ReadInput { addr, count })? {
            Response::Input(values) => Ok(values),
            other => Err(unexpected("0x04", &other)),
        }
    }

    /// Writes one coil.
    pub fn write_coil(&mut self, addr: u16, value: bool) -> Result<(), TcpError> {
        match self.transact(&Request::WriteCoil { addr, value })? {
            Response::WrittenSingle { .. } => Ok(()),
            other => Err(unexpected("0x05", &other)),
        }
    }

    /// Writes one holding register.
    pub fn write_register(&mut self, addr: u16, value: u16) -> Result<(), TcpError> {
        match self.transact(&Request::WriteRegister { addr, value })? {
            Response::WrittenSingle { .. } => Ok(()),
            other => Err(unexpected("0x06", &other)),
        }
    }

    /// Writes several coils from `addr`.
    pub fn write_coils(&mut self, addr: u16, values: &[bool]) -> Result<(), TcpError> {
        match self.transact(&Request::WriteCoils {
            addr,
            values: values.to_vec(),
        })? {
            Response::WrittenMultiple { .. } => Ok(()),
            other => Err(unexpected("0x0F", &other)),
        }
    }

    /// Writes several holding registers from `addr`.
    pub fn write_registers(&mut self, addr: u16, values: &[u16]) -> Result<(), TcpError> {
        match self.transact(&Request::WriteRegisters {
            addr,
            values: values.to_vec(),
        })? {
            Response::WrittenMultiple { .. } => Ok(()),
            other => Err(unexpected("0x10", &other)),
        }
    }

    fn transact(&mut self, request: &Request) -> Result<Response, TcpError> {
        let mut last = TcpError::Transport("no attempts ran".to_owned());
        for _ in 0..=self.retries {
            let transaction = self.next_transaction;
            self.next_transaction = self.next_transaction.wrapping_add(1);
            match self.attempt(transaction, request) {
                Ok(response) => return Ok(response),
                // The slave answered: the same request would fail again.
                Err(TcpError::Exception { function, code }) => {
                    return Err(TcpError::Exception { function, code });
                }
                Err(error) => {
                    last = error;
                }
            }
        }
        Err(last)
    }

    fn attempt(&mut self, transaction: u16, request: &Request) -> Result<Response, TcpError> {
        let frame = encode_request(transaction, self.unit_id, request);
        let mut stream = TcpStream::connect_timeout(&self.addr, self.timeout)
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        stream
            .write_all(&frame)
            .map_err(|error| TcpError::Transport(error.to_string()))?;

        let mut header = [0u8; 7];
        stream
            .read_exact(&mut header)
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        let length = u16::from_be_bytes([header[4], header[5]]) as usize;
        if length < 2 {
            return Err(TcpError::Protocol(format!("MBAP length {length}")));
        }
        let mut rest = vec![0u8; length - 1];
        stream
            .read_exact(&mut rest)
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        let mut full = Vec::with_capacity(7 + rest.len());
        full.extend_from_slice(&header);
        full.extend_from_slice(&rest);

        let (echo_transaction, echo_unit, pdu) = decode_response(&full)?;
        if echo_transaction != transaction {
            return Err(TcpError::Protocol(format!(
                "transaction echo 0x{echo_transaction:04X} != 0x{transaction:04X}"
            )));
        }
        if echo_unit != self.unit_id {
            return Err(TcpError::Protocol(format!(
                "unit echo {} != {}",
                echo_unit, self.unit_id
            )));
        }
        decode_pdu(request, &pdu)
    }
}

/// In-memory Modbus slave exposing the wire side of a [`ModbusMap`](crate::ModbusMap).
///
/// Coils and discrete inputs hold `digital_count` channels; the register banks
/// hold exactly the configured register runs. [`SlaveStore::handle`] validates
/// every request against those banks, so an unmapped address answers
/// `IllegalDataAddress` instead of panicking.
#[derive(Debug, Clone, PartialEq)]
pub struct SlaveStore {
    map: crate::ModbusMap,
    coils: Vec<bool>,
    discrete: Vec<bool>,
    holding: Vec<u16>,
    input: Vec<u16>,
}

impl SlaveStore {
    /// Creates a zeroed store sized by `map`.
    pub fn new(map: crate::ModbusMap) -> Self {
        let coils = vec![false; map.digital_count];
        let discrete = vec![false; map.digital_count];
        let holding = vec![0u16; map.analog_out.register_len()];
        let input = vec![0u16; map.analog_in.register_len()];
        Self {
            map,
            coils,
            discrete,
            holding,
            input,
        }
    }

    /// Configured map.
    pub fn map(&self) -> &crate::ModbusMap {
        &self.map
    }

    /// Coil bank (outputs on the wire).
    pub fn coils(&self) -> &[bool] {
        &self.coils
    }

    /// Discrete input bank (inputs on the wire).
    pub fn discrete(&self) -> &[bool] {
        &self.discrete
    }

    /// Holding register bank.
    pub fn holding(&self) -> &[u16] {
        &self.holding
    }

    /// Input register bank.
    pub fn input(&self) -> &[u16] {
        &self.input
    }

    /// Sets a discrete input, as field wiring would.
    pub fn set_discrete(&mut self, index: usize, value: bool) {
        if let Some(slot) = self.discrete.get_mut(index) {
            *slot = value;
        }
    }

    /// Sets an input register, as field wiring would.
    pub fn set_input_register(&mut self, index: usize, value: u16) {
        if let Some(slot) = self.input.get_mut(index) {
            *slot = value;
        }
    }

    /// Copies the wire inputs into a process image through the map.
    pub fn sync_inputs_to_image(&self, image: &mut crate::IoImage) {
        self.map
            .decode_discrete(&self.discrete, &mut image.digital_in);
        self.map.decode_registers(&self.input, &mut image.analog_in);
    }

    /// Copies a process image into the wire output banks through the map.
    pub fn sync_outputs_from_image(&mut self, image: &crate::IoImage) {
        let coils = self.map.encode_coils(&image.digital_out);
        let width = self.coils.len().min(coils.len());
        self.coils[..width].copy_from_slice(&coils[..width]);
        let holding = self.map.encode_registers(&image.analog_out);
        let width = self.holding.len().min(holding.len());
        self.holding[..width].copy_from_slice(&holding[..width]);
    }

    /// Answers one request, validating addresses against the map.
    pub fn handle(&mut self, request: &Request) -> Result<Response, u8> {
        match request {
            Request::ReadCoils { addr, count } => {
                let range = self.coil_range(*addr, *count, 2000)?;
                Ok(Response::Coils(self.coils[range].to_vec()))
            }
            Request::ReadDiscrete { addr, count } => {
                let range = self.discrete_range(*addr, *count, 2000)?;
                Ok(Response::Discrete(self.discrete[range].to_vec()))
            }
            Request::ReadHolding { addr, count } => {
                let range = self.holding_range(*addr, *count, 125)?;
                Ok(Response::Holding(self.holding[range].to_vec()))
            }
            Request::ReadInput { addr, count } => {
                let range = self.input_range(*addr, *count, 125)?;
                Ok(Response::Input(self.input[range].to_vec()))
            }
            Request::WriteCoil { addr, value } => {
                let index = self.coil_index(*addr)?;
                self.coils[index] = *value;
                Ok(Response::WrittenSingle {
                    function: 0x05,
                    addr: *addr,
                    value: if *value { 0xFF00 } else { 0x0000 },
                })
            }
            Request::WriteRegister { addr, value } => {
                let index = self.holding_index(*addr)?;
                self.holding[index] = *value;
                Ok(Response::WrittenSingle {
                    function: 0x06,
                    addr: *addr,
                    value: *value,
                })
            }
            Request::WriteCoils { addr, values } => {
                if values.is_empty() || values.len() > 1968 {
                    return Err(ExceptionCode::IllegalDataValue as u8);
                }
                let range = self.coil_range(*addr, values.len() as u16, 1968)?;
                self.coils[range].copy_from_slice(values);
                Ok(Response::WrittenMultiple {
                    function: 0x0F,
                    addr: *addr,
                    count: values.len() as u16,
                })
            }
            Request::WriteRegisters { addr, values } => {
                if values.is_empty() || values.len() > 123 {
                    return Err(ExceptionCode::IllegalDataValue as u8);
                }
                let range = self.holding_range(*addr, values.len() as u16, 123)?;
                self.holding[range].copy_from_slice(values);
                Ok(Response::WrittenMultiple {
                    function: 0x10,
                    addr: *addr,
                    count: values.len() as u16,
                })
            }
        }
    }

    fn coil_index(&self, addr: u16) -> Result<usize, u8> {
        let base = u32::from(self.map.coil_base);
        let index = u32::from(addr)
            .checked_sub(base)
            .ok_or(ExceptionCode::IllegalDataAddress as u8)?;
        if (index as usize) < self.coils.len() {
            Ok(index as usize)
        } else {
            Err(ExceptionCode::IllegalDataAddress as u8)
        }
    }

    fn coil_range(&self, addr: u16, count: u16, max: u16) -> Result<std::ops::Range<usize>, u8> {
        check_count(count, max)?;
        let start = self.coil_index(addr)?;
        let end = start.saturating_add(count as usize);
        if end <= self.coils.len() {
            Ok(start..end)
        } else {
            Err(ExceptionCode::IllegalDataAddress as u8)
        }
    }

    fn discrete_range(
        &self,
        addr: u16,
        count: u16,
        max: u16,
    ) -> Result<std::ops::Range<usize>, u8> {
        check_count(count, max)?;
        let base = u32::from(self.map.discrete_base);
        let start = u32::from(addr)
            .checked_sub(base)
            .ok_or(ExceptionCode::IllegalDataAddress as u8)? as usize;
        let end = start.saturating_add(count as usize);
        if end <= self.discrete.len() {
            Ok(start..end)
        } else {
            Err(ExceptionCode::IllegalDataAddress as u8)
        }
    }

    fn holding_index(&self, addr: u16) -> Result<usize, u8> {
        let base = u32::from(self.map.analog_out.register_base);
        let index = u32::from(addr)
            .checked_sub(base)
            .ok_or(ExceptionCode::IllegalDataAddress as u8)?;
        if (index as usize) < self.holding.len() {
            Ok(index as usize)
        } else {
            Err(ExceptionCode::IllegalDataAddress as u8)
        }
    }

    fn holding_range(&self, addr: u16, count: u16, max: u16) -> Result<std::ops::Range<usize>, u8> {
        check_count(count, max)?;
        let start = self.holding_index(addr)?;
        let end = start.saturating_add(count as usize);
        if end <= self.holding.len() {
            Ok(start..end)
        } else {
            Err(ExceptionCode::IllegalDataAddress as u8)
        }
    }

    fn input_range(&self, addr: u16, count: u16, max: u16) -> Result<std::ops::Range<usize>, u8> {
        check_count(count, max)?;
        let base = u32::from(self.map.analog_in.register_base);
        let start = u32::from(addr)
            .checked_sub(base)
            .ok_or(ExceptionCode::IllegalDataAddress as u8)? as usize;
        let end = start.saturating_add(count as usize);
        if end <= self.input.len() {
            Ok(start..end)
        } else {
            Err(ExceptionCode::IllegalDataAddress as u8)
        }
    }
}

/// Serves up to `max_requests` Modbus TCP transactions on `stream`.
///
/// Each request is answered from `store` (or with an exception frame); the
/// MBAP transaction id and unit id are echoed. Returns how many requests ran.
/// A clean client close after at least one request ends the loop normally.
pub fn serve_stream(
    stream: TcpStream,
    store: &mut SlaveStore,
    max_requests: usize,
) -> Result<usize, TcpError> {
    let mut stream = stream;
    let mut served = 0;
    while served < max_requests {
        let mut header = [0u8; 7];
        match stream.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) => {
                if served > 0 && error.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(TcpError::Transport(error.to_string()));
            }
        }
        let length = u16::from_be_bytes([header[4], header[5]]) as usize;
        if !(2..=260).contains(&length) {
            return Err(TcpError::Protocol(format!("MBAP length {length}")));
        }
        let mut rest = vec![0u8; length - 1];
        stream
            .read_exact(&mut rest)
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        let mut full = Vec::with_capacity(7 + rest.len());
        full.extend_from_slice(&header);
        full.extend_from_slice(&rest);

        let transaction = u16::from_be_bytes([full[0], full[1]]);
        let unit = full[6];
        let frame = match decode_request(&full) {
            Ok((_, _, request)) => match store.handle(&request) {
                Ok(response) => encode_response(transaction, unit, &response),
                Err(code) => encode_exception(transaction, unit, request.function(), code),
            },
            Err(TcpError::Exception { function, code }) => {
                encode_exception(transaction, unit, function, code)
            }
            Err(TcpError::Transport(message)) => return Err(TcpError::Transport(message)),
            Err(TcpError::Protocol(message)) => return Err(TcpError::Protocol(message)),
        };
        stream
            .write_all(&frame)
            .map_err(|error| TcpError::Transport(error.to_string()))?;
        served += 1;
    }
    Ok(served)
}

pub(crate) fn decode_pdu(request: &Request, pdu: &[u8]) -> Result<Response, TcpError> {
    let function = pdu[0];
    if function != request.function() {
        return Err(TcpError::Protocol(format!(
            "function echo 0x{function:02X} != 0x{:02X}",
            request.function()
        )));
    }
    match request {
        Request::ReadCoils { count, .. } => {
            let bytes = counted_bytes(pdu, function)?;
            let values = unpack_coils(bytes, *count as usize);
            if values.len() != *count as usize {
                return Err(TcpError::Protocol("coil payload truncated".to_owned()));
            }
            Ok(Response::Coils(values))
        }
        Request::ReadDiscrete { count, .. } => {
            let bytes = counted_bytes(pdu, function)?;
            let values = unpack_coils(bytes, *count as usize);
            if values.len() != *count as usize {
                return Err(TcpError::Protocol("discrete payload truncated".to_owned()));
            }
            Ok(Response::Discrete(values))
        }
        Request::ReadHolding { count, .. } | Request::ReadInput { count, .. } => {
            let bytes = counted_bytes(pdu, function)?;
            if bytes.len() != *count as usize * 2 {
                return Err(TcpError::Protocol("register payload truncated".to_owned()));
            }
            let values = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            if function == 0x03 {
                Ok(Response::Holding(values))
            } else {
                Ok(Response::Input(values))
            }
        }
        Request::WriteCoil { .. } | Request::WriteRegister { .. } => {
            if pdu.len() != 5 {
                return Err(TcpError::Protocol("write echo truncated".to_owned()));
            }
            Ok(Response::WrittenSingle {
                function,
                addr: u16::from_be_bytes([pdu[1], pdu[2]]),
                value: u16::from_be_bytes([pdu[3], pdu[4]]),
            })
        }
        Request::WriteCoils { .. } | Request::WriteRegisters { .. } => {
            if pdu.len() != 5 {
                return Err(TcpError::Protocol("write echo truncated".to_owned()));
            }
            Ok(Response::WrittenMultiple {
                function,
                addr: u16::from_be_bytes([pdu[1], pdu[2]]),
                count: u16::from_be_bytes([pdu[3], pdu[4]]),
            })
        }
    }
}

fn counted_bytes(pdu: &[u8], function: u8) -> Result<&[u8], TcpError> {
    let announced = *pdu.get(1).ok_or_else(|| {
        TcpError::Protocol(format!("function 0x{function:02X} without byte count"))
    })? as usize;
    let bytes = pdu
        .get(2..)
        .ok_or_else(|| TcpError::Protocol("response PDU too short".to_owned()))?;
    if bytes.len() != announced {
        return Err(TcpError::Protocol(format!(
            "byte count {announced} != payload {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn split_frame(frame: &[u8]) -> Result<(u16, u8, &[u8]), TcpError> {
    if frame.len() < 8 {
        return Err(TcpError::Protocol(
            "frame shorter than MBAP + function".to_owned(),
        ));
    }
    let transaction = u16::from_be_bytes([frame[0], frame[1]]);
    let protocol = u16::from_be_bytes([frame[2], frame[3]]);
    if protocol != 0 {
        return Err(TcpError::Protocol(format!("protocol id {protocol}")));
    }
    let length = u16::from_be_bytes([frame[4], frame[5]]) as usize;
    if length < 2 {
        return Err(TcpError::Protocol(format!("MBAP length {length}")));
    }
    if frame.len() != 6 + length {
        return Err(TcpError::Protocol(format!(
            "frame {} bytes != MBAP length {length}",
            frame.len()
        )));
    }
    Ok((transaction, frame[6], &frame[7..]))
}

fn two_u16(body: &[u8], function: u8) -> Result<(u16, u16), TcpError> {
    if body.len() != 4 {
        return Err(TcpError::Protocol(format!(
            "function 0x{function:02X} needs 4 bytes, got {}",
            body.len()
        )));
    }
    Ok((
        u16::from_be_bytes([body[0], body[1]]),
        u16::from_be_bytes([body[2], body[3]]),
    ))
}

fn multi_coils(body: &[u8]) -> Result<(u16, Vec<bool>), TcpError> {
    if body.len() < 5 {
        return Err(TcpError::Protocol("function 0x0F truncated".to_owned()));
    }
    let addr = u16::from_be_bytes([body[0], body[1]]);
    let count = u16::from_be_bytes([body[2], body[3]]) as usize;
    let announced = body[4] as usize;
    let data = &body[5..];
    if data.len() != announced || announced != packed_len(count) {
        return Err(TcpError::Protocol(
            "function 0x0F byte count mismatch".to_owned(),
        ));
    }
    Ok((addr, unpack_coils(data, count)))
}

fn multi_registers(body: &[u8]) -> Result<(u16, Vec<u16>), TcpError> {
    if body.len() < 5 {
        return Err(TcpError::Protocol("function 0x10 truncated".to_owned()));
    }
    let addr = u16::from_be_bytes([body[0], body[1]]);
    let count = u16::from_be_bytes([body[2], body[3]]) as usize;
    let announced = body[4] as usize;
    let data = &body[5..];
    if data.len() != announced || announced != count.saturating_mul(2) {
        return Err(TcpError::Protocol(
            "function 0x10 byte count mismatch".to_owned(),
        ));
    }
    Ok((
        addr,
        data.chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect(),
    ))
}

fn packed_len(count: usize) -> usize {
    count.div_ceil(8)
}

fn push_u16(into: &mut Vec<u8>, value: u16) {
    into.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn wrap_frame(transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(7 + pdu.len());
    frame.extend_from_slice(&transaction.to_be_bytes());
    frame.extend_from_slice(&0u16.to_be_bytes());
    frame.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
    frame.push(unit);
    frame.extend_from_slice(pdu);
    frame
}

fn check_count(count: u16, max: u16) -> Result<(), u8> {
    if count == 0 || count > max {
        Err(ExceptionCode::IllegalDataValue as u8)
    } else {
        Ok(())
    }
}

fn unexpected(expected: &str, got: &Response) -> TcpError {
    TcpError::Protocol(format!("{expected} answered by {got:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modbus::AnalogMap;
    use crate::{AnalogKind, ModbusMap, WordOrder};

    fn test_map() -> ModbusMap {
        ModbusMap {
            coil_base: 100,
            discrete_base: 200,
            digital_count: 8,
            analog_in: AnalogMap::new(0, 300, 2, AnalogKind::I16, 1.0, 0.0, WordOrder::BigEndian),
            analog_out: AnalogMap::new(0, 400, 2, AnalogKind::I16, 1.0, 0.0, WordOrder::BigEndian),
        }
    }

    #[test]
    fn coil_packing_is_lsb_first_and_round_trips() {
        let values = vec![true, false, true, true, false, false, false, false, true];
        let packed = pack_coils(&values);
        assert_eq!(packed, vec![0x0D, 0x01]);
        assert_eq!(unpack_coils(&packed, values.len()), values);
        assert!(unpack_coils(&[], 0).is_empty());
    }

    #[test]
    fn requests_round_trip_through_the_codec() {
        let cases = [
            Request::ReadCoils {
                addr: 100,
                count: 8,
            },
            Request::ReadDiscrete {
                addr: 200,
                count: 3,
            },
            Request::ReadHolding {
                addr: 400,
                count: 2,
            },
            Request::ReadInput {
                addr: 300,
                count: 2,
            },
            Request::WriteCoil {
                addr: 101,
                value: true,
            },
            Request::WriteRegister {
                addr: 400,
                value: 0x1234,
            },
            Request::WriteCoils {
                addr: 100,
                values: vec![true, false, true],
            },
            Request::WriteRegisters {
                addr: 400,
                values: vec![1, 2, 3],
            },
        ];
        for (index, request) in cases.iter().enumerate() {
            let frame = encode_request(index as u16, 1, request);
            let (transaction, unit, decoded) = decode_request(&frame).expect("codec round-trips");
            assert_eq!(transaction, index as u16);
            assert_eq!(unit, 1);
            assert_eq!(&decoded, request);
        }
    }

    #[test]
    fn responses_round_trip_through_the_codec() {
        let request = Request::ReadHolding {
            addr: 400,
            count: 2,
        };
        let response = SlaveStore::new(test_map())
            .handle(&request)
            .expect("mapped read");
        let frame = encode_response(3, 1, &response);
        let (transaction, _, pdu) = decode_response(&frame).expect("response decodes");
        assert_eq!(transaction, 3);
        let back = decode_pdu(&request, &pdu).expect("pdu decodes");
        assert_eq!(back, response);
    }

    #[test]
    fn unknown_functions_decode_as_illegal_function() {
        let frame = wrap_frame(1, 1, &[0x2B, 0x00, 0x00]);
        match decode_request(&frame) {
            Err(TcpError::Exception { function, code }) => {
                assert_eq!(function, 0x2B);
                assert_eq!(code, ExceptionCode::IllegalFunction as u8);
            }
            other => panic!("expected illegal-function, got {other:?}"),
        }
    }

    #[test]
    fn write_single_coil_rejects_other_values() {
        let mut frame = encode_request(
            7,
            1,
            &Request::WriteCoil {
                addr: 100,
                value: true,
            },
        );
        // Corrupt the value half (last two bytes) to 0x0001.
        let last = frame.len() - 1;
        frame[last] = 0x01;
        assert!(matches!(decode_request(&frame), Err(TcpError::Protocol(_))));
    }

    #[test]
    fn malformed_frames_are_rejected() {
        assert!(matches!(decode_request(&[]), Err(TcpError::Protocol(_))));
        let mut frame = encode_request(1, 1, &Request::ReadCoils { addr: 0, count: 1 });
        frame[2] = 0x00;
        frame[3] = 0x01; // protocol id 1
        assert!(matches!(decode_request(&frame), Err(TcpError::Protocol(_))));
        let mut truncated = encode_request(1, 1, &Request::ReadHolding { addr: 0, count: 1 });
        truncated.pop();
        assert!(matches!(
            decode_request(&truncated),
            Err(TcpError::Protocol(_))
        ));
    }

    #[test]
    fn exceptions_round_trip_as_errors() {
        let frame = encode_exception(9, 1, 0x03, ExceptionCode::IllegalDataAddress as u8);
        match decode_response(&frame) {
            Err(TcpError::Exception { function, code }) => {
                assert_eq!(function, 0x03);
                assert_eq!(code, 0x02);
            }
            other => panic!("expected exception, got {other:?}"),
        }
    }

    #[test]
    fn slave_answers_reads_and_validates_addresses() {
        let mut store = SlaveStore::new(test_map());
        store.set_discrete(0, true);
        store.set_input_register(1, 0x1234);

        let response = store
            .handle(&Request::ReadDiscrete {
                addr: 200,
                count: 2,
            })
            .expect("mapped discrete reads");
        assert_eq!(response, Response::Discrete(vec![true, false]));

        let response = store
            .handle(&Request::ReadInput {
                addr: 300,
                count: 2,
            })
            .expect("mapped input reads");
        assert_eq!(response, Response::Input(vec![0, 0x1234]));

        let missing = store.handle(&Request::ReadCoils { addr: 0, count: 1 });
        assert_eq!(missing, Err(ExceptionCode::IllegalDataAddress as u8));

        let overflowing = store.handle(&Request::ReadHolding {
            addr: 401,
            count: 2,
        });
        assert_eq!(overflowing, Err(ExceptionCode::IllegalDataAddress as u8));

        let empty = store.handle(&Request::ReadCoils {
            addr: 100,
            count: 0,
        });
        assert_eq!(empty, Err(ExceptionCode::IllegalDataValue as u8));
    }

    #[test]
    fn slave_writes_land_in_the_banks() {
        let mut store = SlaveStore::new(test_map());
        store
            .handle(&Request::WriteCoil {
                addr: 100,
                value: true,
            })
            .expect("coil writes");
        assert!(store.coils()[0]);
        store
            .handle(&Request::WriteCoils {
                addr: 101,
                values: vec![true, true],
            })
            .expect("multi-coil writes");
        assert_eq!(&store.coils()[1..3], &[true, true]);
        store
            .handle(&Request::WriteRegister {
                addr: 400,
                value: 42,
            })
            .expect("register writes");
        assert_eq!(store.holding()[0], 42);
        store
            .handle(&Request::WriteRegisters {
                addr: 400,
                values: vec![7, 8],
            })
            .expect("multi-register writes");
        assert_eq!(store.holding(), &[7, 8]);

        let outside = store.handle(&Request::WriteCoil {
            addr: 9999,
            value: true,
        });
        assert_eq!(outside, Err(ExceptionCode::IllegalDataAddress as u8));
    }

    #[test]
    fn slave_syncs_with_the_process_image_through_the_map() {
        let mut image = crate::IoImage::new(8, 2);
        image.set_digital_output(0, true).expect("in range");
        image.set_analog_output(1, 9.0).expect("in range");
        let mut store = SlaveStore::new(test_map());
        store.sync_outputs_from_image(&image);
        assert!(store.coils()[0]);
        assert_eq!(store.holding()[1], 9);

        store.set_discrete(2, true);
        store.set_input_register(0, 5);
        store.sync_inputs_to_image(&mut image);
        assert!(image.digital_input(2).unwrap_or(false));
        assert_eq!(image.analog_input(0), Some(5.0));
    }

    #[test]
    fn master_talks_to_a_loopback_slave() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let server = std::thread::spawn(move || {
            let mut store = SlaveStore::new(test_map());
            store.set_discrete(1, true);
            store.set_input_register(0, 0x00AB);
            // The master opens a fresh connection per call, so accept one
            // connection per request instead of serving all on the first.
            for _ in 0..6 {
                let (stream, _) = listener.accept().expect("master connects");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout sets");
                serve_stream(stream, &mut store, 1).expect("slave serves");
            }
            store
        });

        let mut master = TcpMaster::new(addr, 1, Duration::from_secs(5), 1);
        master.write_coil(100, true).expect("coil writes");
        assert_eq!(
            master.read_coils(100, 2).expect("coils read"),
            vec![true, false]
        );
        assert_eq!(
            master.read_discrete(200, 2).expect("discrete read"),
            vec![false, true]
        );
        master.write_register(400, 0x1234).expect("register writes");
        assert_eq!(
            master.read_holding(400, 1).expect("holding read"),
            vec![0x1234]
        );
        assert_eq!(master.read_input(300, 1).expect("input read"), vec![0x00AB]);

        let store = server.join().expect("server joins");
        assert!(store.coils()[0]);
        assert_eq!(store.holding()[0], 0x1234);
    }

    #[test]
    fn master_surfaces_exceptions_without_retrying() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("master connects");
            let mut store = SlaveStore::new(test_map());
            // One request: the unmapped read answered with an exception.
            serve_stream(stream, &mut store, 1).expect("slave serves");
        });

        let mut master = TcpMaster::new(addr, 1, Duration::from_secs(5), 3);
        match master.read_coils(0, 1) {
            Err(TcpError::Exception { function, code }) => {
                assert_eq!(function, 0x01);
                assert_eq!(code, ExceptionCode::IllegalDataAddress as u8);
            }
            other => panic!("expected exception, got {other:?}"),
        }
        server.join().expect("server joins");
    }
}
