pub mod aead;
pub mod seed;

/// Length-independent, branch-free byte comparison. Used when checking an
/// entered seed phrase against the stored one so that a wrong guess cannot be
/// narrowed down by timing.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
