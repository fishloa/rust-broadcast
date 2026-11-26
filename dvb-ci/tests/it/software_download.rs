//! DSM-CC U-N Download messages (ISO/IEC 13818-6 Tables 79-83, carried by ETSI
//! TS 101 699 §6.7): exact wire bytes, serialize-length returns, field offsets,
//! truncation sweeps and length-field ceilings.

use broadcast_core::error::BufferTooShort;
use broadcast_core::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::ci_ext::software_download::{
    DownloadCancel, DownloadDataBlock, DownloadDataRequest, DownloadInfoRequest,
    DownloadInfoResponse,
};

const INFO_REQUEST_LEN: usize = 38;
const INFO_RESPONSE_LEN: usize = 65;
const CANCEL_LEN: usize = 32;
const DATA_REQUEST_LEN: usize = 21;
const DATA_BLOCK_LEN: usize = 120;

fn header(message_id: u16, id: u32, adaptation: &[u8], message_length: usize) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&[0x11, 0x03, (message_id >> 8) as u8, message_id as u8]);
    wire.extend_from_slice(&id.to_be_bytes());
    wire.push(0xFF);
    wire.push(adaptation.len() as u8);
    wire.extend_from_slice(&(message_length as u16).to_be_bytes());
    wire.extend_from_slice(adaptation);
    wire
}

fn block_data() -> Vec<u8> {
    (0u8..100).map(|i| i ^ 0x5A).collect()
}

fn info_request() -> Vec<u8> {
    let mut wire = header(0x1001, 0x0001_0203, &[0xA1, 0xA2, 0xA3, 0xA4, 0xA5], 26);
    wire.extend_from_slice(&0x1122_3344u32.to_be_bytes());
    wire.extend_from_slice(&0x5566u16.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB]);
    wire.extend_from_slice(&[0x00, 0x07]);
    wire.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45]);
    wire
}

fn info_response() -> Vec<u8> {
    let mut wire = header(0x1002, 0x0A0B_0C0D, &[0xB1, 0xB2, 0xB3], 53);
    wire.extend_from_slice(&0x0102_0304u32.to_be_bytes());
    wire.extend_from_slice(&0x0506u16.to_be_bytes());
    wire.extend_from_slice(&[0x07, 0x08]);
    wire.extend_from_slice(&0x090A_0B0Cu32.to_be_bytes());
    wire.extend_from_slice(&0x0D0E_0F10u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB]);
    wire.extend_from_slice(&[
        0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x01, 0x01, 0xFF, 0x00, 0x02, 0x00, 0x00,
        0x00, 0x20, 0x02, 0x00,
    ]);
    wire.extend_from_slice(&[0x00, 0x07]);
    wire.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45]);
    wire
}

fn cancel() -> Vec<u8> {
    let mut wire = header(0x1005, 0x0A0B_0C0D, &[0xC1, 0xC2], 20);
    wire.extend_from_slice(&0x0102_0304u32.to_be_bytes());
    wire.extend_from_slice(&0x0506u16.to_be_bytes());
    wire.extend_from_slice(&0x0708u16.to_be_bytes());
    wire.push(0x09);
    wire.extend_from_slice(&[0x00, 0x07]);
    wire.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45]);
    wire
}

fn data_request() -> Vec<u8> {
    let mut wire = header(0x1004, 0x1122_3344, &[0xD1, 0xD2, 0xD3, 0xD4], 9);
    wire.extend_from_slice(&0x5566u16.to_be_bytes());
    wire.extend_from_slice(&0x7788u16.to_be_bytes());
    wire.push(0x99);
    wire
}

fn data_block() -> Vec<u8> {
    let mut wire = header(0x1003, 0x0102_0304, &[0xE1, 0xE2], 108);
    wire.extend_from_slice(&0x0506u16.to_be_bytes());
    wire.push(0x07);
    wire.push(0xFF);
    wire.extend_from_slice(&0x0809u16.to_be_bytes());
    wire.extend_from_slice(&block_data());
    wire
}

fn serialize_len<T: Serialize<Error = Error>>(value: &T) -> (usize, Vec<u8>) {
    let mut buf = vec![0u8; value.serialized_len()];
    let n = value.serialize_into(&mut buf).expect("serialize");
    (n, buf)
}

fn patched_prefix(full: &[u8], cut: usize) -> Vec<u8> {
    let mut wire = full[..cut].to_vec();
    if cut >= 12 {
        wire[10..12].copy_from_slice(&((cut - 12) as u16).to_be_bytes());
    }
    wire
}

macro_rules! expect_short {
    ($ty:ty, $wire:expr, $need:expr, $have:expr) => {
        match <$ty>::parse($wire) {
            Err(Error::BufferTooShort(BufferTooShort {
                need: got_need,
                have: got_have,
                ..
            })) => assert_eq!((got_need, got_have), ($need, $have)),
            other => panic!(
                "expected BufferTooShort({},{}), got {:?}",
                $need, $have, other
            ),
        }
    };
}

#[test]
fn download_info_request_exact_fields_and_return() {
    let wire = info_request();
    assert_eq!(wire.len(), INFO_REQUEST_LEN);
    let parsed = DownloadInfoRequest::parse(&wire).unwrap();
    assert_eq!(parsed.transaction_id, 0x0001_0203);
    assert_eq!(parsed.adaptation, &[0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
    assert_eq!(parsed.buffer_size, 0x1122_3344);
    assert_eq!(parsed.maximum_block_size, 0x5566);
    assert_eq!(
        parsed.compatibility_descriptor,
        &[0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB]
    );
    assert_eq!(
        parsed.private_data,
        &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45]
    );

    let value = DownloadInfoRequest {
        transaction_id: 0x0001_0203,
        adaptation: &[0xA1, 0xA2, 0xA3, 0xA4, 0xA5],
        buffer_size: 0x1122_3344,
        maximum_block_size: 0x5566,
        compatibility_descriptor: &[0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB],
        private_data: &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, INFO_REQUEST_LEN);
    assert_eq!(n, value.serialized_len());
    assert_eq!(buf, wire);
    assert_eq!(
        &buf[..12],
        &[
            0x11, 0x03, 0x10, 0x01, 0x00, 0x01, 0x02, 0x03, 0xFF, 0x05, 0x00, 0x1A
        ]
    );
}

#[test]
fn download_info_response_exact_fields_and_return() {
    let wire = info_response();
    assert_eq!(wire.len(), INFO_RESPONSE_LEN);
    let parsed = DownloadInfoResponse::parse(&wire).unwrap();
    assert_eq!(parsed.transaction_id, 0x0A0B_0C0D);
    assert_eq!(parsed.adaptation, &[0xB1, 0xB2, 0xB3]);
    assert_eq!(parsed.download_id, 0x0102_0304);
    assert_eq!(parsed.block_size, 0x0506);
    assert_eq!(parsed.window_size, 0x07);
    assert_eq!(parsed.ack_period, 0x08);
    assert_eq!(parsed.tc_download_window, 0x090A_0B0C);
    assert_eq!(parsed.tc_download_scenario, 0x0D0E_0F10);
    assert_eq!(
        parsed.compatibility_descriptor,
        &[0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB]
    );
    assert_eq!(parsed.modules.len(), 19);
    assert_eq!(&parsed.modules[..2], &[0x00, 0x02]);
    assert_eq!(
        parsed.private_data,
        &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45]
    );

    let modules = &wire[12 + 3 + 16 + 6..12 + 3 + 16 + 6 + 19];
    let value = DownloadInfoResponse {
        transaction_id: 0x0A0B_0C0D,
        adaptation: &[0xB1, 0xB2, 0xB3],
        download_id: 0x0102_0304,
        block_size: 0x0506,
        window_size: 0x07,
        ack_period: 0x08,
        tc_download_window: 0x090A_0B0C,
        tc_download_scenario: 0x0D0E_0F10,
        compatibility_descriptor: &[0x00, 0x04, 0x00, 0x00, 0xAA, 0xBB],
        modules,
        private_data: &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, INFO_RESPONSE_LEN);
    assert_eq!(n, value.serialized_len());
    assert_eq!(buf, wire);
    assert_eq!(
        &buf[..12],
        &[
            0x11, 0x03, 0x10, 0x02, 0x0A, 0x0B, 0x0C, 0x0D, 0xFF, 0x03, 0x00, 0x35
        ]
    );
}

#[test]
fn download_cancel_exact_fields_and_return() {
    let wire = cancel();
    assert_eq!(wire.len(), CANCEL_LEN);
    let parsed = DownloadCancel::parse(&wire).unwrap();
    assert_eq!(parsed.transaction_id, 0x0A0B_0C0D);
    assert_eq!(parsed.adaptation, &[0xC1, 0xC2]);
    assert_eq!(parsed.download_id, 0x0102_0304);
    assert_eq!(parsed.module_id, 0x0506);
    assert_eq!(parsed.block_number, 0x0708);
    assert_eq!(parsed.download_cancel_reason, 0x09);
    assert_eq!(
        parsed.private_data,
        &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45]
    );

    let value = DownloadCancel {
        transaction_id: 0x0A0B_0C0D,
        adaptation: &[0xC1, 0xC2],
        download_id: 0x0102_0304,
        module_id: 0x0506,
        block_number: 0x0708,
        download_cancel_reason: 0x09,
        private_data: &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, CANCEL_LEN);
    assert_eq!(n, value.serialized_len());
    assert_eq!(buf, wire);
}

#[test]
fn download_data_request_exact_fields_and_return() {
    let wire = data_request();
    assert_eq!(wire.len(), DATA_REQUEST_LEN);
    let parsed = DownloadDataRequest::parse(&wire).unwrap();
    assert_eq!(parsed.download_id, 0x1122_3344);
    assert_eq!(parsed.adaptation, &[0xD1, 0xD2, 0xD3, 0xD4]);
    assert_eq!(parsed.module_id, 0x5566);
    assert_eq!(parsed.block_number, 0x7788);
    assert_eq!(parsed.download_reason, 0x99);

    let value = DownloadDataRequest {
        download_id: 0x1122_3344,
        adaptation: &[0xD1, 0xD2, 0xD3, 0xD4],
        module_id: 0x5566,
        block_number: 0x7788,
        download_reason: 0x99,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, DATA_REQUEST_LEN);
    assert_eq!(n, value.serialized_len());
    assert_eq!(buf, wire);
}

#[test]
fn download_data_block_exact_fields_and_return() {
    let wire = data_block();
    assert_eq!(wire.len(), DATA_BLOCK_LEN);
    let parsed = DownloadDataBlock::parse(&wire).unwrap();
    assert_eq!(parsed.download_id, 0x0102_0304);
    assert_eq!(parsed.adaptation, &[0xE1, 0xE2]);
    assert_eq!(parsed.module_id, 0x0506);
    assert_eq!(parsed.module_version, 0x07);
    assert_eq!(parsed.block_number, 0x0809);
    assert_eq!(parsed.block_data, block_data().as_slice());

    let data = block_data();
    let value = DownloadDataBlock {
        download_id: 0x0102_0304,
        adaptation: &[0xE1, 0xE2],
        module_id: 0x0506,
        module_version: 0x07,
        block_number: 0x0809,
        block_data: &data,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, DATA_BLOCK_LEN);
    assert_eq!(n, value.serialized_len());
    assert_eq!(buf, wire);
}

#[test]
fn every_truncated_info_request_prefix_is_rejected() {
    let wire = info_request();
    for cut in 0..wire.len() {
        assert!(
            DownloadInfoRequest::parse(&patched_prefix(&wire, cut)).is_err(),
            "cut {cut}"
        );
    }
    expect_short!(DownloadInfoRequest, &patched_prefix(&wire, 12), 17, 12);
    expect_short!(DownloadInfoRequest, &patched_prefix(&wire, 30), 14, 13);
    expect_short!(DownloadInfoRequest, &patched_prefix(&wire, 25), 14, 8);
    expect_short!(DownloadInfoRequest, &patched_prefix(&wire, 31), 21, 14);
}

#[test]
fn every_truncated_info_response_prefix_reports_the_fixed_need() {
    let wire = info_response();
    for cut in 0..wire.len() {
        assert!(
            DownloadInfoResponse::parse(&patched_prefix(&wire, cut)).is_err(),
            "cut {cut}"
        );
    }
    for rest_len in 0..18 {
        let cut = 12 + 3 + rest_len;
        expect_short!(
            DownloadInfoResponse,
            &patched_prefix(&wire, cut),
            18,
            rest_len
        );
    }
    expect_short!(DownloadInfoResponse, &patched_prefix(&wire, 12), 15, 12);
    for (rest_len, need) in [
        (18usize, 24usize),
        (24, 32),
        (32, 33),
        (33, 41),
        (42, 43),
        (43, 50),
    ] {
        let cut = 12 + 3 + rest_len;
        expect_short!(
            DownloadInfoResponse,
            &patched_prefix(&wire, cut),
            need,
            rest_len
        );
    }
}

#[test]
fn every_truncated_cancel_prefix_reports_the_fixed_need() {
    let wire = cancel();
    for cut in 0..wire.len() {
        assert!(
            DownloadCancel::parse(&patched_prefix(&wire, cut)).is_err(),
            "cut {cut}"
        );
    }
    for rest_len in 0..11 {
        let cut = 12 + 2 + rest_len;
        expect_short!(DownloadCancel, &patched_prefix(&wire, cut), 11, rest_len);
    }
    expect_short!(DownloadCancel, &patched_prefix(&wire, 12), 14, 12);
    expect_short!(DownloadCancel, &patched_prefix(&wire, 25), 18, 11);
}

#[test]
fn every_truncated_data_request_prefix_reports_the_fixed_need() {
    let wire = data_request();
    for cut in 0..wire.len() {
        assert!(
            DownloadDataRequest::parse(&patched_prefix(&wire, cut)).is_err(),
            "cut {cut}"
        );
    }
    expect_short!(DownloadDataRequest, &patched_prefix(&wire, 16 + 4), 5, 4);
    expect_short!(DownloadDataRequest, &patched_prefix(&wire, 12), 16, 12);
}

#[test]
fn every_truncated_data_block_prefix_below_the_fixed_part_is_rejected() {
    let wire = data_block();
    for cut in 0..20 {
        assert!(
            DownloadDataBlock::parse(&patched_prefix(&wire, cut)).is_err(),
            "cut {cut}"
        );
    }
    expect_short!(DownloadDataBlock, &patched_prefix(&wire, 19), 6, 5);
    expect_short!(DownloadDataBlock, &patched_prefix(&wire, 12), 14, 12);
    let empty = patched_prefix(&wire, 20);
    assert_eq!(
        DownloadDataBlock::parse(&empty).unwrap().block_data,
        &[] as &[u8]
    );
}
