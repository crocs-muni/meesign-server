pub mod gg18;
use prost::Message;
use crate::proto::Gg18Message;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ProtocolType {
    GG18,
}

struct Communicator {
    parties: usize,
    input: Vec<Vec<Option<Vec<u8>>>>,
    output: Vec<Vec<u8>>,
}

impl Communicator {
    pub fn new(parties: usize) -> Self {
        Communicator {
            parties,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        for _ in 0..self.parties {
            self.input.push(vec![None; self.parties]);
        }
    }

    pub fn broadcast(&mut self, message: Vec<u8>) {
        self.output.clear();

        for _ in 0..self.parties {
            self.output.push(message.clone());
        }
    }

    pub fn relay(&mut self) {
        self.output.clear();

        for i in 0..self.parties {
            let mut out: Vec<Vec<u8>> = Vec::new();
            for j in 0..self.parties {
                if i != j {
                    out.push(self.input[j][i].clone().unwrap());
                }
            }
            let message = Gg18Message { message: out };
            self.output.push(message.encode_to_vec());
        }
    }

    pub fn send_all<F>(&mut self, f: F) where F: Fn(usize) -> Vec<u8> {
        self.output.clear();

        for i in 0..self.parties {
            self.output.push(f(i));
        }
    }

    pub fn round_received(&self, ids: &[Vec<u8>]) -> bool {
        ids.into_iter()
            .zip((&self.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count() == 0
    }

    pub fn get_message(&self, idx: usize) -> Option<&Vec<u8>> {
        self.output.get(idx)
    }
}
