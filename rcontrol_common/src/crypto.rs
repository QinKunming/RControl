//! 口令认证 + 会话密钥协商 (PSK 模式, 无需证书)
//!
//! 流程:
//! 1. 服务端 -> 客户端: ServerHello{ nonce_s }
//! 2. 双方用 password + nonce_c + nonce_s 派生 auth_key / session_key
//! 3. 客户端 -> 服务端: AuthRequest{ nonce_c, proof=HMAC(auth_key,"client-proof") }
//! 4. 服务端校验后回 AuthResponse{ ok, proof=HMAC(auth_key,"server-proof") }
//! 5. 之后所有帧用 session_key 做 AES-256-GCM 加密 (计数器 nonce)

use aes_gcm::aead::consts::U12;
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::Aes256Gcm;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use sha2::Sha256;

type Nonce12 = GenericArray<u8, U12>;

pub const NONCE_LEN: usize = 16;
const AUTH_INFO: &[u8] = b"rcontrol-auth-v1";
const SESSION_INFO: &[u8] = b"rcontrol-session-v1";
const CLIENT_PROOF_LABEL: &[u8] = b"rcontrol-client-proof-v1";
const SERVER_PROOF_LABEL: &[u8] = b"rcontrol-server-proof-v1";

pub type AuthKey = [u8; 32];

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// 由口令和双方随机数派生 (auth_key, session_key)
pub fn derive_keys(password: &str, nonce_c: &[u8], nonce_s: &[u8]) -> (AuthKey, [u8; 32]) {
    let mut salt = Vec::with_capacity(nonce_c.len() + nonce_s.len());
    salt.extend_from_slice(nonce_c);
    salt.extend_from_slice(nonce_s);
    let hk = Hkdf::<Sha256>::new(Some(&salt), password.as_bytes());
    let mut auth_key = [0u8; 32];
    let mut session_key = [0u8; 32];
    hk.expand(AUTH_INFO, &mut auth_key).expect("hkdf auth");
    hk.expand(SESSION_INFO, &mut session_key).expect("hkdf session");
    (auth_key, session_key)
}

pub fn random_nonce() -> Vec<u8> {
    let mut b = vec![0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b
}

pub fn client_proof(auth_key: &AuthKey) -> Vec<u8> {
    hmac_sha256(auth_key, CLIENT_PROOF_LABEL)
}

pub fn server_proof(auth_key: &AuthKey) -> Vec<u8> {
    hmac_sha256(auth_key, SERVER_PROOF_LABEL)
}

pub fn verify_client_proof(auth_key: &AuthKey, proof: &[u8]) -> bool {
    // 常量时间比较
    let expect = hmac_sha256(auth_key, CLIENT_PROOF_LABEL);
    if expect.len() != proof.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expect.iter().zip(proof.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// AES-256-GCM 加密通道。收发方向各自维护计数器作为 nonce。
pub struct Channel {
    cipher: Aes256Gcm,
    enc_ctr: u64,
    dec_ctr: u64,
}

impl Channel {
    pub fn new(session_key: &[u8; 32]) -> Self {
        Channel {
            cipher: Aes256Gcm::new_from_slice(session_key).expect("key len"),
            enc_ctr: 0,
            dec_ctr: 0,
        }
    }

    fn ctr_nonce(ctr: u64) -> Nonce12 {
        let mut n = [0u8; 12];
        n[4..12].copy_from_slice(&ctr.to_be_bytes());
        GenericArray::from(n)
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let out = self
            .cipher
            .encrypt(&Self::ctr_nonce(self.enc_ctr), Payload { msg: plaintext, aad: &[] })
            .expect("aes-gcm seal");
        self.enc_ctr += 1;
        out
    }

    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
        let out = self
            .cipher
            .decrypt(&Self::ctr_nonce(self.dec_ctr), Payload { msg: ciphertext, aad: &[] })
            .map_err(|_| ())?;
        self.dec_ctr += 1;
        Ok(out)
    }
}

/// 生成随机口令 (字母数字, 去掉易混淆字符)
pub fn generate_password(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::rngs::OsRng;
    (0..len)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len()) as usize] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_roundtrip() {
        let pw = "secret-pw-1";
        let nc = random_nonce();
        let ns = random_nonce();
        let (ak_c, sk_c) = derive_keys(pw, &nc, &ns);
        let (ak_s, sk_s) = derive_keys(pw, &nc, &ns);
        assert_eq!(ak_c, ak_s);
        assert!(verify_client_proof(&ak_s, &client_proof(&ak_c)));
        assert!(!verify_client_proof(&ak_s, b"bad"));
        assert_eq!(server_proof(&ak_c), server_proof(&ak_s));

        let mut a = Channel::new(&sk_c);
        let mut b = Channel::new(&sk_s);
        let ct = a.seal(b"hello");
        assert_eq!(b.open(&ct).unwrap(), b"hello");
        let ct = b.seal(b"world");
        assert_eq!(a.open(&ct).unwrap(), b"world");
        // 错位解密失败
        assert!(a.open(b"junk").is_err());
    }
}
