//! 配对令牌（AGENT_DESIGN.md §五）：高熵随机令牌，展示给管理员一次，Agent 只存
//! SHA-256 哈希，不落盘明文。令牌是"一次配对多次使用"，不是一次性——校验只是简单的
//! 哈希比对，暴力破解 256-bit 熵的随机值不现实，不需要 argon2 这类慢哈希。

use rand::RngCore;
use sha2::{Digest, Sha256};

const TOKEN_BYTES: usize = 20; // 160 bit，Base32 编码后是 32 个字符，分 4 组展示

/// 生成形如 `roc-agent-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX` 的令牌
/// （Base32 不区分大小写、排除易混淆字符，适合让人口头/IM 转述）。
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let encoded = data_encoding::BASE32_NOPAD.encode(&bytes);
    let groups: Vec<String> = encoded.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect();
    format!("roc-agent-{}", groups.join("-"))
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.trim().as_bytes());
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

pub fn verify_token(token: &str, expected_hash: &str) -> bool {
    // 定长哈希做逐字节异或累加，不在第一个不匹配字节就提前返回——避免时序侧信道
    // 泄露"哈希匹配到第几位"，虽然这里的攻击面（本地网络内的一次握手）风险有限，
    // 但这个写法零成本，没有理由不做。
    let actual = hash_token(token);
    if actual.len() != expected_hash.len() {
        return false;
    }
    actual.bytes().zip(expected_hash.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}
