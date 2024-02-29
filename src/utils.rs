pub fn hextrunc<T: AsRef<[u8]>>(s: T) -> String {
    let trunc_len = std::env::var("TRUNC")
        .ok()
        .as_ref()
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    if trunc_len == 0 {
        hex::encode(s.as_ref())
    } else {
        format!("{}...", hex::encode(&s.as_ref()[..trunc_len]))
    }
}
