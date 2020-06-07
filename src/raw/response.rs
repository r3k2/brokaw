use std::borrow::Cow;
use std::str::{from_utf8, from_utf8_unchecked};

use crate::types::response_code::ResponseCode;

/// A response returned by the low-level [`NntpConnection`](super::connection::NntpConnection)
///
/// 1. The contents are guaranteed to be represent a syntactically valid NNTP response
/// 2. The contents ARE NOT guaranteed to be UTF-8 as the NNTP does not require contents be UTF-8.
#[derive(Clone, Debug)]
pub struct RawResponse {
    pub(crate) code: ResponseCode,
    pub(crate) first_line: Vec<u8>,
    pub(crate) data_blocks: Option<DataBlocks>,
}

impl RawResponse {
    /// The response code
    pub fn code(&self) -> ResponseCode {
        self.code
    }

    /// Return true if this response is a multi-line response and contains a data block section
    pub fn has_data_blocks(&self) -> bool {
        match self.data_blocks {
            Some(_) => true,
            _ => false,
        }
    }

    /// Return multi-line data blocks
    pub fn data_blocks(&self) -> Option<&DataBlocks> {
        self.data_blocks.as_ref()
    }

    /// Return the first line of the response
    pub fn first_line(&self) -> &[u8] {
        &self.first_line
    }

    /// Return the first line of the response without the response code
    pub fn first_line_without_code(&self) -> &[u8] {
        // n.b. this should be infallible barring bugs in the response parsing layer
        &self.first_line[4..]
    }

    /// Return the first line as UTF-8
    pub fn first_line_as_utf8(&self) -> Result<&str, std::str::Utf8Error> {
        from_utf8(&self.first_line).map(|s| s.trim())
    }

    /// Lossily convert the first line to UTF-8
    pub fn first_line_to_utf8_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.first_line)
    }

    /// Convert the initial response payload into UTF-8 without checking
    ///
    /// # Safety
    ///
    /// This function is unsafe because NNTP responses are NOT required to be UTF-8.
    /// This call simply calls [`from_utf8_unchecked`] under the hood.
    ///
    pub unsafe fn first_line_as_utf8_unchecked(&self) -> &str {
        from_utf8_unchecked(&self.first_line)
    }
}

/// The [Multi-line Data Blocks](https://tools.ietf.org/html/rfc3977#section-3.1.1)
/// portion of an NNTP response
///
/// * [`DataBlocks::iter`] returns an iterator over the lines within the block
/// * [`DataBlocks::payload`] returns the raw bytes
#[derive(Clone, Debug)]
pub struct DataBlocks {
    pub(crate) payload: Vec<u8>,
    pub(crate) line_boundaries: Vec<(usize, usize)>,
}

impl DataBlocks {
    /// Return the raw contained by the payload of the Datablocks
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// A convenience function that simply calls [`from_utf8`]
    pub fn payload_as_utf8(&self) -> Result<&str, std::str::Utf8Error> {
        from_utf8(&self.payload)
    }

    /// An iterator over the lines within the data block
    pub fn lines(&self) -> Lines<'_> {
        Lines {
            data_blocks: self,
            inner: self.line_boundaries.iter(),
        }
    }

    // FIXME(ux): Consider introducing an iterator that does not include CRLF terminators

    /// The number of lines
    pub fn lines_len(&self) -> usize {
        self.line_boundaries.len()
    }

    /// The number of bytes in the data block
    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    /// Returns true if there are no lines
    pub fn is_empty(&self) -> bool {
        self.line_boundaries.is_empty()
    }
}

/// An iterator over the data blocks within a response
pub struct Lines<'a> {
    data_blocks: &'a DataBlocks,
    inner: std::slice::Iter<'a, (usize, usize)>,
}

impl<'a> Iterator for Lines<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((start, end)) = self.inner.next() {
            Some(&self.data_blocks.payload[*start..*end])
        } else {
            None
        }
    }
}
