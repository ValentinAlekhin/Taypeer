//! Pull-based authenticated chunks; callers publish only after verified EOF.

use crate::{Error, ReadKey, crypto};
use std::io::{self, Read};
use zeroize::Zeroizing;

pub(super) struct DecryptReader<'a, R> {
    input: R,
    key: &'a ReadKey,
    header: Vec<u8>,
    remaining: u64,
    index: u64,
    buffer: Zeroizing<Vec<u8>>,
    offset: usize,
    done: bool,
}

impl<'a, R: Read> DecryptReader<'a, R> {
    pub(super) fn new(input: R, key: &'a ReadKey, header: Vec<u8>) -> Result<Self, Error> {
        let remaining = crypto::payload_length(&header)?;
        if remaining > crypto::MAX_PAYLOAD {
            return Err(Error::TooLarge);
        }
        Ok(Self {
            input,
            key,
            header,
            remaining,
            index: 0,
            buffer: Zeroizing::new(Vec::new()),
            offset: 0,
            done: false,
        })
    }
    fn next(&mut self) -> Result<(), Error> {
        let final_chunk = self.remaining == 0;
        let size = self.remaining.min(crypto::CHUNK as u64) as usize;
        let mut nonce = [0; 24];
        self.input.read_exact(&mut nonce)?;
        let mut cipher = vec![0; size + 16];
        self.input.read_exact(&mut cipher)?;
        self.buffer = crypto::open(
            self.key,
            &nonce,
            &crypto::chunk_aad(&self.header, self.index, final_chunk),
            &cipher,
        )?;
        self.offset = 0;
        self.remaining -= size as u64;
        self.index += 1;
        if final_chunk {
            let mut extra = [0];
            if self.input.read(&mut extra)? != 0 {
                return Err(Error::InvalidFile);
            }
            self.done = true;
        }
        Ok(())
    }
}
impl<R: Read> Read for DecryptReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        if self.offset == self.buffer.len() {
            self.next().map_err(io::Error::other)?;
        }
        let size = output.len().min(self.buffer.len() - self.offset);
        output[..size].copy_from_slice(&self.buffer[self.offset..self.offset + size]);
        self.offset += size;
        Ok(size)
    }
}
