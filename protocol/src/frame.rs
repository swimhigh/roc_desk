//! 应用层二进制分帧（AGENT_DESIGN.md §3.2）：单条 TLS 连接承载多个逻辑"流"，
//! 复用 SSH 那边"一条物理连接、多个 Channel"的多路复用思路，但协议本身简单得多——
//! 不需要信道流控，直接用一个"长度 + 流 ID + 帧类型 + payload"的定长头部分帧。
//!
//! ```text
//! ┌──────────┬──────────┬──────────┬────────────────────┐
//! │ 长度(u32) │ 流ID(u32) │ 帧类型(u8)│      Payload        │
//! │  4 bytes │  4 bytes │  1 byte  │   (长度 bytes)      │
//! └──────────┴──────────┴──────────┴────────────────────┘
//! ```

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const FRAME_HEADER_LEN: usize = 4 + 4 + 1;

/// `长度` 字段只含 payload 长度，不含头部本身；协议里最大的合法 payload 是文件/
/// 终端数据块，发送方按 `DATA_CHUNK_SIZE` 主动切片，不需要单帧超过这个上限——
/// 设一个上限是为了不让一个损坏/恶意的长度前缀诱使读端分配荒谬大小的缓冲区。
pub const MAX_FRAME_PAYLOAD_LEN: u32 = 16 * 1024 * 1024;

/// `DataChunk` 帧发送方按这个大小切片大文件/命令输出，不是协议强制要求，只是
/// 客户端/Agent 双方约定的默认值，避免单帧过大占用过多内存。
pub const DATA_CHUNK_SIZE: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Payload 是 JSON 编码的 `Request`/`Response`，用于所有"一问一答"的 RPC。
    Control = 0x01,
    /// Payload 是裸二进制字节（大文件读写/终端字节流）——不经过 JSON，避免
    /// base64 编码带来的体积膨胀和序列化开销。
    DataChunk = 0x02,
    /// 标记某个流的数据结束（`ReadFile` 分块读完、`WriteFile` 内容发送完毕）。
    StreamEnd = 0x03,
    /// 流级别错误，比 Control 帧里携带的业务错误更底层（比如流 ID 冲突）。
    Error = 0x04,
}

impl FrameType {
    fn from_u8(b: u8) -> io::Result<Self> {
        match b {
            0x01 => Ok(FrameType::Control),
            0x02 => Ok(FrameType::DataChunk),
            0x03 => Ok(FrameType::StreamEnd),
            0x04 => Ok(FrameType::Error),
            other => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown frame type: {other}"))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub stream_id: u32,
    pub frame_type: FrameType,
    pub payload: Vec<u8>,
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    stream_id: u32,
    frame_type: FrameType,
    payload: &[u8],
) -> io::Result<()> {
    if payload.len() as u64 > MAX_FRAME_PAYLOAD_LEN as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame payload exceeds MAX_FRAME_PAYLOAD_LEN"));
    }
    let mut header = [0u8; FRAME_HEADER_LEN];
    header[0..4].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    header[4..8].copy_from_slice(&stream_id.to_be_bytes());
    header[8] = frame_type as u8;
    w.write_all(&header).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Frame> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    r.read_exact(&mut header).await?;
    let len = u32::from_be_bytes(header[0..4].try_into().unwrap());
    if len > MAX_FRAME_PAYLOAD_LEN {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame payload exceeds MAX_FRAME_PAYLOAD_LEN"));
    }
    let stream_id = u32::from_be_bytes(header[4..8].try_into().unwrap());
    let frame_type = FrameType::from_u8(header[8])?;
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).await?;
    Ok(Frame { stream_id, frame_type, payload })
}
