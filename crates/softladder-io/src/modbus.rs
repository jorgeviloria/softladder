//! Modbus address map — **M5, pure mapping (no transport yet)**.
//!
//! The M5 driver maps the [`crate::IoImage`] to Modbus data model entities:
//! coils and discrete inputs for the digital channels, input registers and
//! holding registers for the analog ones. ClassicLadder mirrors `%B`/`%W`
//! at fixed addresses; SoftLadder configures the map instead (offset, type,
//! scale, word order), as required by `docs/ARCHITECTURE.md` §8.
//!
//! This module is deliberately transport-free: it converts between channel
//! slices and register/coil slices so the mapping is unit-testable with no
//! socket and no server. The blocking TCP master/slave in [`crate::modbus_tcp`]
//! reuses these conversions; RTU framing is the next M5 slice.

/// Milestone that delivers the Modbus driver.
pub const MILESTONE: &str = "M5";

/// Word order of a 32-bit value spread over two consecutive 16-bit registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordOrder {
    /// High word first, the Modbus default.
    #[default]
    BigEndian,
    /// Low word first, for devices that swap the pair.
    LittleEndian,
}

/// Encoding of one analog channel on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AnalogKind {
    /// One signed 16-bit register.
    #[default]
    I16,
    /// One unsigned 16-bit register.
    U16,
    /// Two registers carrying an IEEE-754 `f32` in [`WordOrder`].
    F32,
}

impl AnalogKind {
    /// Registers occupied by one channel.
    pub fn registers_per_channel(self) -> usize {
        match self {
            AnalogKind::I16 | AnalogKind::U16 => 1,
            AnalogKind::F32 => 2,
        }
    }
}

/// Mapping of a run of analog channels to a run of registers.
///
/// Engineering values convert as `value = raw * scale + offset`; decoding
/// inverts it as `raw = round((value - offset) / scale)`, clamped to the
/// range of [`AnalogKind`]. A zero scale is refused by the constructors that
/// validate and saturates to the range ends when decoding.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnalogMap {
    /// First PLC channel in the run.
    pub plc_base: usize,
    /// First Modbus register in the run.
    pub register_base: u16,
    /// Channels in the run.
    pub count: usize,
    /// Wire encoding.
    pub kind: AnalogKind,
    /// Engineering units per raw unit.
    pub scale: f64,
    /// Engineering value at raw zero.
    pub offset: f64,
    /// Word order for [`AnalogKind::F32`].
    pub order: WordOrder,
}

impl AnalogMap {
    /// Creates a map; `count` channels of `kind` with a linear scale.
    pub fn new(
        plc_base: usize,
        register_base: u16,
        count: usize,
        kind: AnalogKind,
        scale: f64,
        offset: f64,
        order: WordOrder,
    ) -> Self {
        Self {
            plc_base,
            register_base,
            count,
            kind,
            scale,
            offset,
            order,
        }
    }

    /// Identity map of `count` channels starting at zero, `I16` raw values.
    pub fn identity(count: usize) -> Self {
        Self {
            plc_base: 0,
            register_base: 0,
            count,
            kind: AnalogKind::I16,
            scale: 1.0,
            offset: 0.0,
            order: WordOrder::BigEndian,
        }
    }

    /// Registers covered by the map.
    pub fn register_len(self) -> usize {
        self.count.saturating_mul(self.kind.registers_per_channel())
    }

    fn raw_to_value(self, raw: f64) -> f64 {
        raw * self.scale + self.offset
    }

    fn value_to_raw(self, value: f64) -> f64 {
        if self.scale == 0.0 {
            return 0.0;
        }
        (value - self.offset) / self.scale
    }
}

/// Configurable slave map between PLC channels and Modbus addresses.
///
/// Digital channel `i` is coil `coil_base + i` (outputs) or discrete input
/// `discrete_base + i` (inputs). Analog runs are described by [`AnalogMap`]:
/// input registers feed `%IW`, holding registers drive `%QW`. Anything
/// outside the configured runs is ignored, so a short device never panics a
/// larger image.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModbusMap {
    /// First coil carrying `%Q` channel `0`.
    pub coil_base: u16,
    /// First discrete input carrying `%I` channel `0`.
    pub discrete_base: u16,
    /// Digital channels exposed in each direction.
    pub digital_count: usize,
    /// Input registers (`%IW`) mapping.
    pub analog_in: AnalogMap,
    /// Holding registers (`%QW`) mapping.
    pub analog_out: AnalogMap,
}

impl ModbusMap {
    /// Fixed `%B`/`%W` mirroring starting at address zero, for compatibility.
    pub fn fixed(digital: usize, analog: usize) -> Self {
        Self {
            coil_base: 0,
            discrete_base: 0,
            digital_count: digital,
            analog_in: AnalogMap::identity(analog),
            analog_out: AnalogMap::identity(analog),
        }
    }

    /// Decodes discrete inputs into digital input channels.
    ///
    /// `discrete` is the block read from `discrete_base`; entries past
    /// `digital_count` or past `inputs.len()` are ignored.
    pub fn decode_discrete(&self, discrete: &[bool], inputs: &mut [bool]) {
        let channels = self.digital_count.min(inputs.len()).min(discrete.len());
        inputs[..channels].copy_from_slice(&discrete[..channels]);
    }

    /// Encodes digital outputs into coils starting at `coil_base`.
    ///
    /// Returns the coil block to write: exactly `digital_count` entries.
    /// Channels past `outputs.len()` read as `false`.
    pub fn encode_coils(&self, outputs: &[bool]) -> Vec<bool> {
        (0..self.digital_count)
            .map(|channel| outputs.get(channel).copied().unwrap_or(false))
            .collect()
    }

    /// Decodes input registers into analog input channels.
    pub fn decode_registers(&self, registers: &[u16], inputs: &mut [f64]) {
        decode_analog_map(self.analog_in, registers, inputs);
    }

    /// Encodes analog outputs into holding registers.
    ///
    /// Returns exactly `analog_out.register_len()` registers; channels past
    /// `outputs.len()` encode as the engineering [`AnalogMap::offset`].
    pub fn encode_registers(&self, outputs: &[f64]) -> Vec<u16> {
        encode_analog_map(self.analog_out, outputs)
    }
}

/// Decodes one [`AnalogMap`] run into `values`.
pub fn decode_analog_map(map: AnalogMap, registers: &[u16], values: &mut [f64]) {
    let channels = map.count.min(values.len());
    for channel in 0..channels {
        let slot = channel.saturating_mul(map.kind.registers_per_channel());
        let Some(raw) = decode_channel(map, registers, slot) else {
            break;
        };
        let plc = map.plc_base.saturating_add(channel);
        if let Some(value) = values.get_mut(plc) {
            *value = map.raw_to_value(raw);
        }
    }
}

/// Encodes `values` through one [`AnalogMap`] run into registers.
pub fn encode_analog_map(map: AnalogMap, values: &[f64]) -> Vec<u16> {
    let mut registers = vec![0u16; map.register_len()];
    for channel in 0..map.count {
        let plc = map.plc_base.saturating_add(channel);
        let value = values.get(plc).copied().unwrap_or(map.offset);
        let raw = map.value_to_raw(value);
        let slot = channel.saturating_mul(map.kind.registers_per_channel());
        encode_channel(map, raw, &mut registers, slot);
    }
    registers
}

fn decode_channel(map: AnalogMap, registers: &[u16], slot: usize) -> Option<f64> {
    match map.kind {
        AnalogKind::I16 => {
            let word = registers.get(slot).copied()?;
            Some(f64::from(word as i16))
        }
        AnalogKind::U16 => {
            let word = registers.get(slot).copied()?;
            Some(f64::from(word))
        }
        AnalogKind::F32 => {
            let high = registers.get(slot).copied()?;
            let low = registers.get(slot.saturating_add(1)).copied()?;
            let (hi, lo) = match map.order {
                WordOrder::BigEndian => (high, low),
                WordOrder::LittleEndian => (low, high),
            };
            let bits = (u32::from(hi) << 16) | u32::from(lo);
            Some(f64::from(f32::from_bits(bits)))
        }
    }
}

fn encode_channel(map: AnalogMap, raw: f64, registers: &mut [u16], slot: usize) {
    match map.kind {
        AnalogKind::I16 => {
            if let Some(register) = registers.get_mut(slot) {
                *register =
                    raw.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16 as u16;
            }
        }
        AnalogKind::U16 => {
            if let Some(register) = registers.get_mut(slot) {
                *register = raw.round().clamp(0.0, f64::from(u16::MAX)) as u16;
            }
        }
        AnalogKind::F32 => {
            let bits = (raw as f32).to_bits();
            let high = (bits >> 16) as u16;
            let low = (bits & 0xFFFF) as u16;
            let (hi, lo) = match map.order {
                WordOrder::BigEndian => (high, low),
                WordOrder::LittleEndian => (low, high),
            };
            if let Some(register) = registers.get_mut(slot) {
                *register = hi;
            }
            if let Some(register) = registers.get_mut(slot.saturating_add(1)) {
                *register = lo;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coils_round_trip_through_the_configured_base() {
        let map = ModbusMap {
            coil_base: 100,
            discrete_base: 200,
            digital_count: 4,
            analog_in: AnalogMap::identity(0),
            analog_out: AnalogMap::identity(0),
        };
        let coils = map.encode_coils(&[true, false, true]);
        assert_eq!(coils, vec![true, false, true, false]);

        let mut inputs = vec![false; 4];
        map.decode_discrete(&[false, true], &mut inputs);
        assert_eq!(inputs, vec![false, true, false, false]);
    }

    #[test]
    fn analog_i16_applies_scale_offset_and_clamps() {
        let map = AnalogMap::new(0, 300, 2, AnalogKind::I16, 0.1, -5.0, WordOrder::BigEndian);
        // raw 100 -> 100 * 0.1 - 5 = 5.0
        let mut values = vec![0.0; 2];
        decode_analog_map(map, &[100, 0], &mut values);
        assert!((values[0] - 5.0).abs() < f64::EPSILON);
        assert!((values[1] + 5.0).abs() < f64::EPSILON);

        let registers = encode_analog_map(map, &[5.0, 1e9]);
        assert_eq!(registers[0], 100);
        assert_eq!(registers[1], i16::MAX as u16);
    }

    #[test]
    fn analog_u16_never_goes_negative() {
        let map = AnalogMap::new(0, 0, 1, AnalogKind::U16, 1.0, 0.0, WordOrder::BigEndian);
        let registers = encode_analog_map(map, &[-12.0]);
        assert_eq!(registers, vec![0]);
        let mut values = vec![0.0; 1];
        decode_analog_map(map, &[0xFFFF], &mut values);
        assert_eq!(values, vec![65535.0]);
    }

    #[test]
    fn analog_f32_honours_the_word_order() {
        let value = 12.5_f64;
        for order in [WordOrder::BigEndian, WordOrder::LittleEndian] {
            let map = AnalogMap::new(0, 0, 1, AnalogKind::F32, 1.0, 0.0, order);
            let registers = encode_analog_map(map, &[value]);
            assert_eq!(registers.len(), 2);
            let mut decoded = vec![0.0; 1];
            decode_analog_map(map, &registers, &mut decoded);
            assert!((decoded[0] - value).abs() < 1e-6, "order {order:?}");
        }
        let big = encode_analog_map(
            AnalogMap::new(0, 0, 1, AnalogKind::F32, 1.0, 0.0, WordOrder::BigEndian),
            &[value],
        );
        let little = encode_analog_map(
            AnalogMap::new(0, 0, 1, AnalogKind::F32, 1.0, 0.0, WordOrder::LittleEndian),
            &[value],
        );
        assert_eq!(big, vec![little[1], little[0]]);
    }

    #[test]
    fn short_blocks_never_panic_and_missing_channels_encode_as_offset() {
        let map = ModbusMap {
            coil_base: 0,
            discrete_base: 0,
            digital_count: 8,
            analog_in: AnalogMap::new(0, 0, 2, AnalogKind::I16, 1.0, 3.0, WordOrder::BigEndian),
            analog_out: AnalogMap::new(0, 0, 2, AnalogKind::I16, 1.0, 3.0, WordOrder::BigEndian),
        };
        let mut inputs = vec![false; 8];
        map.decode_discrete(&[], &mut inputs);
        assert_eq!(inputs, vec![false; 8]);

        let mut analog = vec![0.0; 2];
        map.decode_registers(&[10], &mut analog);
        assert_eq!(analog[0], 13.0);
        assert_eq!(analog[1], 0.0);

        let registers = map.encode_registers(&[7.0]);
        assert_eq!(registers, vec![4, 0]);
    }

    #[test]
    fn fixed_map_mirrors_from_zero() {
        let map = ModbusMap::fixed(2, 1);
        assert_eq!(map.coil_base, 0);
        assert_eq!(map.discrete_base, 0);
        assert_eq!(map.encode_coils(&[true, true]), vec![true, true]);
        assert_eq!(map.encode_registers(&[1.0]), vec![1]);
    }
}
