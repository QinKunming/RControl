//! 帧编解码: [4 字节大端长度][protobuf Message] / 加密后 [4 字节大端长度][密文]
//! (密文长度 = 明文长度 + 16 字节 GCM tag)

use crate::crypto::Channel;
use crate::proto::Message;
use prost::Message as _;
use std::io::{self, Read, Write};

pub const MAX_FRAME: usize = 32 * 1024 * 1024;
const FILE_CHUNK: usize = 256 * 1024;

/// 文件传输分片大小
pub fn file_chunk_size() -> usize {
    FILE_CHUNK
}

/// 读一个长度前缀帧 (明文)
pub fn read_frame_plain(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame length out of range"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// 写一个长度前缀帧 (明文)
pub fn write_frame_plain(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    if data.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(data)?;
    Ok(())
}

/// 编码 Message 为字节
pub fn encode_msg(msg: &Message) -> Vec<u8> {
    msg.encode_to_vec()
}

/// 从字节解码 Message
pub fn decode_msg(data: &[u8]) -> io::Result<Message> {
    Message::decode(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// 加密并写出一个 Message 帧
pub fn write_msg_enc(w: &mut impl Write, ch: &mut Channel, msg: &Message) -> io::Result<()> {
    let pt = msg.encode_to_vec();
    let ct = ch.seal(&pt);
    write_frame_plain(w, &ct)
}

/// 读取并解密一个 Message 帧
pub fn read_msg_enc(r: &mut impl Read, ch: &mut Channel) -> io::Result<Message> {
    let ct = read_frame_plain(r)?;
    let pt = ch.open(&ct).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "decrypt failed"))?;
    decode_msg(&pt)
}

/// 明文读一个 Message (握手阶段)
pub fn read_msg_plain(r: &mut impl Read) -> io::Result<Message> {
    let buf = read_frame_plain(r)?;
    decode_msg(&buf)
}

/// 明文写一个 Message (握手阶段)
pub fn write_msg_plain(w: &mut impl Write, msg: &Message) -> io::Result<()> {
    write_frame_plain(w, &msg.encode_to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{derive_keys, random_nonce, Channel};
    use crate::proto::*;

    fn chan() -> (Channel, Channel) {
        let nc = random_nonce();
        let ns = random_nonce();
        let (_, sk) = derive_keys("pw", &nc, &ns);
        (Channel::new(&sk), Channel::new(&sk))
    }

    #[test]
    fn frame_roundtrip() {
        let msg = Message { msg: Some(message::Msg::Mouse(MouseEvent { x: 1.5, y: 2.5, mask: MOUSE_MOVE, wheel: 0 })) };
        let mut buf = Vec::new();
        write_msg_plain(&mut buf, &msg).unwrap();
        let back = read_msg_plain(&mut buf.as_slice()).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn enc_roundtrip() {
        let (mut a, mut b) = chan();
        let msg = Message { msg: Some(message::Msg::VideoFrame(VideoFrame { jpeg: vec![1, 2, 3], w: 4, h: 5, cursor_x: -1, cursor_y: -1, cursor_visible: false, on_secure: false })) };
        let mut buf = Vec::new();
        write_msg_enc(&mut buf, &mut a, &msg).unwrap();
        let back = read_msg_enc(&mut buf.as_slice(), &mut b).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn bad_length_rejected() {
        let mut buf: &[u8] = &[0xff, 0xff, 0xff, 0xff, 0, 0];
        assert!(read_frame_plain(&mut buf).is_err());
    }
}
