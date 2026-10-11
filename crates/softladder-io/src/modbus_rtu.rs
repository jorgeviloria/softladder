//! Modbus RTU framing — **M5 slice three (pure codec, no serial transport)**.
//!
//! RTU carries the same PDU as Modbus TCP ([`Request`](crate::modbus_tcp::Request)
//! / [`Response`](crate::modbus_tcp::Response)) with a different frame around it:
//! `[unit | PDU… | CRC-lo | CRC-hi]`. This module implements that framing and the
//! CRC-16/Modbus checksum, reusing the TCP PDU codec so the two transports can
//! never disagree on what a function code means.
//!
//! What is *not* here: the serial line itself. Opening `/dev/ttyS*`, baud rate,
//! and the 3.5-character inter-frame silence need real hardware (or a serial
//! mock) and travel with the GPIO/HAL drivers. [`SlaveStore`](crate::modbus_tcp::SlaveStore)
//! already answers [`Request`](crate::modbus_tcp::Request)s, so a future serial
//! server only needs to feed it frames decoded here.

use crate::modbus_tcp::{
    decode_pdu, decode_request, encode_exception, encode_request, encode_response, Request,
    Response, TcpError,
};

/// CRC-16/Modbus of `data` (poly `0xA001`, init `0xFFFF`).
///
/// The string `b"123456789"` checksums to `0x4B37`.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            if crc & 0x0001 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

/// Encodes a request as an RTU frame for unit `unit`.
pub fn encode_request_rtu(unit: u8, request: &Request) -> Vec<u8> {
    // A TCP frame is `[MBAP(7) | PDU]`; the PDU is shared, so slice it off.
    let tcp = encode_request(0, unit, request);
    let mut frame = Vec::with_capacity(tcp.len());
    frame.push(unit);
    frame.extend_from_slice(&tcp[7..]);
    push_crc(&mut frame);
    frame
}

/// Decodes an RTU request frame into unit and PDU.
///
/// Unknown functions decode as [`TcpError::Exception`] with illegal-function,
/// exactly like the TCP path, so a server answers them the same way.
pub fn decode_request_rtu(frame: &[u8]) -> Result<(u8, Request), TcpError> {
    let (unit, pdu) = split_rtu(frame)?;
    // Reuse the TCP parser through a synthetic MBAP header: the PDU grammar
    // is identical, only the framing differs.
    let length = pdu.len() as u16 + 1;
    let mut synthetic = vec![0u8, 0u8, 0u8, 0u8, (length >> 8) as u8, length as u8, unit];
    synthetic.extend_from_slice(pdu);
    let (_, _, request) = decode_request(&synthetic)?;
    Ok((unit, request))
}

/// Encodes a response PDU as an RTU frame for unit `unit`.
pub fn encode_response_rtu(unit: u8, response: &Response) -> Vec<u8> {
    let tcp = encode_response(0, unit, response);
    let mut frame = Vec::with_capacity(tcp.len());
    frame.push(unit);
    frame.extend_from_slice(&tcp[7..]);
    push_crc(&mut frame);
    frame
}

/// Encodes an exception as an RTU frame for unit `unit`.
pub fn encode_exception_rtu(unit: u8, function: u8, code: u8) -> Vec<u8> {
    let tcp = encode_exception(0, unit, function, code);
    let mut frame = Vec::with_capacity(tcp.len());
    frame.push(unit);
    frame.extend_from_slice(&tcp[7..]);
    push_crc(&mut frame);
    frame
}

/// Decodes an RTU response frame against the `request` that produced it.
///
/// Checks the unit echo and the CRC, then matches the PDU exactly like
/// the TCP master does.
pub fn decode_response_rtu(
    frame: &[u8],
    unit: u8,
    request: &Request,
) -> Result<Response, TcpError> {
    let (echo_unit, pdu) = split_rtu(frame)?;
    if echo_unit != unit {
        return Err(TcpError::Protocol(format!(
            "unit echo {echo_unit} != {unit}"
        )));
    }
    if pdu.len() < 2 {
        return Err(TcpError::Protocol("response PDU too short".to_owned()));
    }
    if pdu[0] & 0x80 != 0 {
        return Err(TcpError::Exception {
            function: pdu[0] & 0x7F,
            code: pdu[1],
        });
    }
    decode_pdu(request, pdu)
}

fn split_rtu(frame: &[u8]) -> Result<(u8, &[u8]), TcpError> {
    if frame.len() < 4 {
        return Err(TcpError::Protocol(
            "RTU frame shorter than unit + function + CRC".to_owned(),
        ));
    }
    let (body, checksum) = frame.split_at(frame.len() - 2);
    let received = u16::from_le_bytes([checksum[0], checksum[1]]);
    let computed = crc16(body);
    if received != computed {
        return Err(TcpError::Protocol(format!(
            "RTU CRC 0x{received:04X} != 0x{computed:04X}"
        )));
    }
    Ok((body[0], &body[1..]))
}

fn push_crc(frame: &mut Vec<u8>) {
    let crc = crc16(frame);
    frame.extend_from_slice(&crc.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modbus_tcp::SlaveStore;
    use crate::{AnalogKind, AnalogMap, ModbusMap, WordOrder};

    fn test_map() -> ModbusMap {
        ModbusMap {
            coil_base: 0,
            discrete_base: 0,
            digital_count: 8,
            analog_in: AnalogMap::identity(2),
            analog_out: AnalogMap::new(0, 0, 2, AnalogKind::I16, 1.0, 0.0, WordOrder::BigEndian),
        }
    }

    #[test]
    fn crc_matches_the_standard_check_vector() {
        assert_eq!(crc16(b"123456789"), 0x4B37);
        assert_eq!(crc16(&[]), 0xFFFF);
    }

    #[test]
    fn requests_round_trip_through_rtu_framing() {
        let cases = [
            Request::ReadCoils { addr: 0, count: 8 },
            Request::ReadDiscrete { addr: 0, count: 3 },
            Request::ReadHolding { addr: 0, count: 2 },
            Request::ReadInput { addr: 0, count: 2 },
            Request::WriteCoil {
                addr: 1,
                value: true,
            },
            Request::WriteRegister {
                addr: 0,
                value: 0x1234,
            },
            Request::WriteCoils {
                addr: 0,
                values: vec![true, false, true],
            },
            Request::WriteRegisters {
                addr: 0,
                values: vec![1, 2, 3],
            },
        ];
        for request in &cases {
            let frame = encode_request_rtu(7, request);
            let (unit, decoded) = decode_request_rtu(&frame).expect("RTU round-trips");
            assert_eq!(unit, 7);
            assert_eq!(&decoded, request);
        }
    }

    #[test]
    fn rtu_and_tcp_carry_the_same_pdu() {
        let request = Request::ReadHolding { addr: 10, count: 3 };
        let tcp = encode_request(0x1234, 5, &request);
        let rtu = encode_request_rtu(5, &request);
        // TCP PDU == RTU body after the unit byte.
        assert_eq!(&tcp[7..], &rtu[1..rtu.len() - 2]);
    }

    #[test]
    fn responses_round_trip_against_the_slave() {
        let mut store = SlaveStore::new(test_map());
        let request = Request::ReadHolding { addr: 0, count: 2 };
        let response = store.handle(&request).expect("mapped read");
        let frame = encode_response_rtu(7, &response);
        let back = decode_response_rtu(&frame, 7, &request).expect("response decodes");
        assert_eq!(back, response);

        let write = Request::WriteCoil {
            addr: 0,
            value: true,
        };
        let response = store.handle(&write).expect("mapped write");
        let frame = encode_response_rtu(7, &response);
        let back = decode_response_rtu(&frame, 7, &write).expect("write echo decodes");
        assert_eq!(back, response);
        assert!(store.coils()[0]);
    }

    #[test]
    fn exceptions_round_trip_as_errors() {
        let frame = encode_exception_rtu(7, 0x03, 0x02);
        match decode_response_rtu(&frame, 7, &Request::ReadHolding { addr: 0, count: 1 }) {
            Err(TcpError::Exception { function, code }) => {
                assert_eq!(function, 0x03);
                assert_eq!(code, 0x02);
            }
            other => panic!("expected exception, got {other:?}"),
        }
    }

    #[test]
    fn corrupted_frames_are_rejected() {
        let mut frame = encode_request_rtu(7, &Request::ReadCoils { addr: 0, count: 1 });
        assert!(decode_request_rtu(&[]).is_err());
        assert!(decode_request_rtu(&frame[..2]).is_err());

        // Flip a payload bit: the CRC must catch it.
        frame[2] ^= 0xFF;
        assert!(matches!(
            decode_request_rtu(&frame),
            Err(TcpError::Protocol(_))
        ));

        // A wrong unit echo is a protocol error, not an exception.
        let response = Response::Holding(vec![1]);
        let frame = encode_response_rtu(7, &response);
        let request = Request::ReadHolding { addr: 0, count: 1 };
        assert!(matches!(
            decode_response_rtu(&frame, 8, &request),
            Err(TcpError::Protocol(_))
        ));
    }
}
