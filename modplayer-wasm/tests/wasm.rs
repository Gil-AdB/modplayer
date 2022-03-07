#![feature(seek_convenience)]
#![feature(seek_stream_len)]

use wasm_bindgen_test::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_browser);

use std::io::{Read, Seek, BufReader, Cursor, SeekFrom};

#[cfg(test)]
#[wasm_bindgen_test]
fn pass() {
    let small_buf = [0u8;10];
    assert_eq!(1, open_module_test(&small_buf));

    let big_buf = [0u8;100];
    assert_eq!(2, open_module_test(&big_buf));
}

pub fn open_module_test(data: &[u8]) -> usize {
    let mut buf = Cursor::new(data);

    return read_data(&mut buf);
}


pub fn read_data<R: Read + Seek>(mut file: &mut R) -> usize {
    file.seek(SeekFrom::Start(0));
    let file_len = match file.stream_len() {
        Ok(m) => { m }
        Err(_) => { return 0; }
    };

    if file_len < 60 {
        return 1;
    }

    return 2;
}