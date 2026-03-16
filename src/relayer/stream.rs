use color_eyre::eyre::eyre;
use ipld_core::ipld::Ipld;
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameHeader {
    Message(Option<String>),
    Error,
}

impl TryFrom<Ipld> for FrameHeader {
    type Error = color_eyre::eyre::Error;

    fn try_from(value: Ipld) -> color_eyre::Result<Self> {
        if let Ipld::Map(map) = value
            && let Some(Ipld::Integer(i)) = map.get("op")
        {
            match i {
                1 => {
                    let t = if let Some(Ipld::String(s)) = map.get("t") {
                        Some(s.clone())
                    } else {
                        None
                    };
                    return Ok(FrameHeader::Message(t));
                }
                -1 => return Ok(FrameHeader::Error),
                _ => {}
            }
        }
        Err(eyre!("invalid frame type"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Message(Option<String>, MessageFrame),
    Error(ErrorFrame),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFrame {
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorFrame {}

impl TryFrom<&[u8]> for Frame {
    type Error = color_eyre::eyre::Error;

    fn try_from(value: &[u8]) -> color_eyre::Result<Self> {
        let mut cursor = Cursor::new(value);
        let (left, right) = match serde_ipld_dagcbor::from_reader::<Ipld, _>(&mut cursor) {
            Err(serde_ipld_dagcbor::DecodeError::TrailingData) => {
                value.split_at(cursor.position() as usize)
            }
            _ => {
                return Err(eyre!("invalid frame type"));
            }
        };
        let header = FrameHeader::try_from(serde_ipld_dagcbor::from_slice::<Ipld>(left)?)?;
        if let FrameHeader::Message(t) = &header {
            Ok(Frame::Message(
                t.clone(),
                MessageFrame {
                    body: right.to_vec(),
                },
            ))
        } else {
            Ok(Frame::Error(ErrorFrame {}))
        }
    }
}
