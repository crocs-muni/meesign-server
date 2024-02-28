pub fn hextrunc<T: AsRef<[u8]>>(s: T) -> String {
    format!("{}...", hex::encode(&s.as_ref()[..4]))
}
