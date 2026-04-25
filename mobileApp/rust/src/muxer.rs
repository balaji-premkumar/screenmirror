use byteorder::{WriteBytesExt, LittleEndian};
use std::sync::mpsc::SyncSender;
use once_cell::sync::Lazy;
use std::sync::Mutex;

static BUFFER_POOL: Lazy<Mutex<Vec<Vec<u8>>>> = Lazy::new(|| {
    let mut pool = Vec::with_capacity(5);
    for _ in 0..5 {
        pool.push(Vec::with_capacity(1024 * 1024)); // 1MB initial
    }
    Mutex::new(pool)
});

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
    tx: SyncSender<Vec<u8>>,
}

impl Muxer {
    pub fn new(tx: SyncSender<Vec<u8>>) -> Self {
        Muxer { tx }
    }

    /// Push raw H.265 / HEVC encoded video data into the mux pipeline
    pub fn push_video(&mut self, h265_data: &[u8]) {
        let frame = Self::frame_packet_pooled(PacketType::Video, h265_data);
        let _ = self.tx.send(frame);
    }

    /// Push raw AAC encoded audio data into the mux pipeline
    pub fn push_audio(&mut self, aac_data: &[u8]) {
        let frame = Self::frame_packet_pooled(PacketType::Audio, aac_data);
        let _ = self.tx.send(frame);
    }

    fn frame_packet_pooled(ptype: PacketType, data: &[u8]) -> Vec<u8> {
        let mut buf = if let Ok(mut pool) = BUFFER_POOL.lock() {
            pool.pop().unwrap_or_else(|| Vec::with_capacity(data.len() + 9))
        } else {
            Vec::with_capacity(data.len() + 9)
        };
        
        buf.clear();
        buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        buf.push(ptype as u8);
        buf.write_u32::<LittleEndian>(data.len() as u32).unwrap();
        buf.extend_from_slice(data);
        buf
    }

    /// Returns a buffer to the pool after use
    pub fn release_buffer(buf: Vec<u8>) {
        if let Ok(mut pool) = BUFFER_POOL.lock() {
            if pool.len() < 10 {
                pool.push(buf);
            }
        }
    }
}

/// Legacy standalone packet struct (kept for compatibility)
pub struct AvPacket {
    pub ptype: PacketType,
    pub data: Vec<u8>,
}

impl AvPacket {
    pub fn serialize(&self) -> Vec<u8> {
        Muxer::frame_packet_pooled(self.ptype, &self.data)
    }
}
