//! Scriptable in-memory transport for tests and offline development.

use super::Transport;
use crate::error::Result;
use std::collections::VecDeque;

/// One recorded operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// A vendor control read.
    ControlIn {
        /// `bRequest`.
        request: u8,
        /// `wValue`.
        value: u16,
        /// Requested length in bytes.
        len: usize,
    },
    /// A vendor control write.
    ControlOut {
        /// `bRequest`.
        request: u8,
        /// `wValue`.
        value: u16,
        /// The bytes sent.
        data: Vec<u8>,
    },
    /// A bulk write (the bytes sent).
    BulkOut(Vec<u8>),
    /// A bulk read.
    BulkIn {
        /// Requested length in bytes.
        len: usize,
    },
}

type ControlInHandler = Box<dyn FnMut(u8, u16, usize) -> Vec<u8> + Send>;
type BulkInHandler = Box<dyn FnMut(&[Call], usize) -> Vec<u8> + Send>;

/// Records every call and answers reads from queues or a handler.
pub struct MockTransport {
    /// Every call made so far, in order.
    pub calls: Vec<Call>,
    control_in_handler: ControlInHandler,
    bulk_in_queue: VecDeque<Vec<u8>>,
    bulk_in_handler: Option<BulkInHandler>,
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTransport {
    /// A mock that answers `0xB2` with "high speed" and everything else with zeros.
    pub fn new() -> Self {
        Self {
            calls: Vec::new(),
            control_in_handler: Box::new(|req, _, len| {
                let mut v = vec![0u8; len];
                if req == 0xB2 && !v.is_empty() {
                    v[0] = 1;
                }
                v
            }),
            bulk_in_queue: VecDeque::new(),
            bulk_in_handler: None,
        }
    }

    /// Replace the control-in responder.
    pub fn on_control_in(&mut self, f: impl FnMut(u8, u16, usize) -> Vec<u8> + Send + 'static) {
        self.control_in_handler = Box::new(f);
    }

    /// Queue the next bulk-in response (consumed before the handler is consulted).
    pub fn push_bulk_in(&mut self, data: Vec<u8>) {
        self.bulk_in_queue.push_back(data);
    }

    /// Responder for bulk reads when the queue is empty; it sees the call history and the requested length.
    pub fn on_bulk_in(&mut self, f: impl FnMut(&[Call], usize) -> Vec<u8> + Send + 'static) {
        self.bulk_in_handler = Some(Box::new(f));
    }

    /// All bulk-out payloads in order.
    pub fn bulk_writes(&self) -> Vec<Vec<u8>> {
        self.calls
            .iter()
            .filter_map(|c| {
                if let Call::BulkOut(d) = c {
                    Some(d.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Transport for MockTransport {
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>> {
        self.calls.push(Call::ControlIn { request, value, len });
        Ok((self.control_in_handler)(request, value, len))
    }

    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.calls.push(Call::ControlOut {
            request,
            value,
            data: data.to_vec(),
        });
        Ok(())
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        self.calls.push(Call::BulkOut(data.to_vec()));
        Ok(())
    }

    fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>> {
        self.calls.push(Call::BulkIn { len });
        if let Some(d) = self.bulk_in_queue.pop_front() {
            return Ok(d);
        }
        if let Some(h) = self.bulk_in_handler.as_mut() {
            return Ok(h(&self.calls, len));
        }
        Ok(vec![0u8; len])
    }

    fn delay(&mut self, _ms: u64) {}
}
