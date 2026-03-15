use byteorder::{WriteBytesExt, LittleEndian};
use std::sync::mpsc::Sender;

/// Frame type tag bytes sent over the USB link
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum PacketType {
    Video = 0x01,
    Audio = 0x02,
}

/// Muxer that interleaves video and audio frames and sends them
/// over a channel for the USB writer to transmit.
pub struct Muxer {
    tx: Sender<Vec<u8>>,
}

impl Muxer {
    pub fn new(tx: Sender<Vec<u8>>) -> Self {
        Muxer { tx }
    }

    /// Push raw H.265 / HEVC encoded video data into the mux pipeline
    pub fn push_video(&mut self, h265_data: &[u8]) {
        let frame = Self::frame_packet(PacketType::Video, h265_data);
        let _ = self.tx.send(frame);
    }

    /// Push raw AAC encoded audio data into the mux pipeline
    pub fn push_audio(&mut self, aac_data: &[u8]) {
        let frame = Self::frame_packet(PacketType::Audio, aac_data);
        let _ = self.tx.send(frame);
    }

    /// Create a framed packet with magic header for reliable deserialization
    /// Wire format: [4B magic][1B type][4B length LE][NB data]
    fn frame_packet(ptype: PacketType, data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(data.len() + 9);
        // Magic header: 0xDE 0xAD 0xBE 0xEF — for frame synchronization on the receiver
        buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        buf.push(ptype as u8);
        buf.write_u32::<LittleEndian>(data.len() as u32).unwrap();
        buf.extend_from_slice(data);
        buf
    }
}

/// Legacy standalone packet struct (kept for compatibility)
pub struct AvPacket {
    pub ptype: PacketType,
    pub data: Vec<u8>,
}

impl AvPacket {
    pub fn serialize(&self) -> Vec<u8> {
        Muxer::frame_packet(self.ptype, &self.data)
    }
}
