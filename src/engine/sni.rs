//! Извлечение SNI из TLS ClientHello и Host из HTTP-запроса.
//!
//! Используется для hostlist-матчинга правил профилей. Чистый Rust,
//! без FFI; усечённые/нестандартные пакеты → None (правило не сработает,
//! пакет уйдёт passthrough — безопасная деградация).

/// Результат разбора SNI: само имя и его смещение в payload
/// (смещение нужно split-позициям sniext+1 / midsld).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sni {
    pub host: String,
    /// Смещение начала имени хоста в payload.
    pub name_off: usize,
}

/// Извлечь SNI из TLS ClientHello payload (RFC 6066, server_name ext).
pub fn extract_tls_sni(payload: &[u8]) -> Option<Sni> {
    // TLS record: type(1) ver(2) len(2); handshake: type(1) len(3).
    if payload.len() < 9 || payload[0] != 0x16 || payload[5] != 0x01 {
        return None;
    }
    let mut p = 5 + 4; // handshake header
    p += 2 + 32; // client_version + random
                 // session_id
    let sid_len = *payload.get(p)? as usize;
    p += 1 + sid_len;
    // cipher_suites
    let cs_len = u16::from_be_bytes([*payload.get(p)?, *payload.get(p + 1)?]) as usize;
    p += 2 + cs_len;
    // compression_methods
    let cm_len = *payload.get(p)? as usize;
    p += 1 + cm_len;
    // extensions
    let ext_total = u16::from_be_bytes([*payload.get(p)?, *payload.get(p + 1)?]) as usize;
    p += 2;
    let ext_end = (p + ext_total).min(payload.len());
    while p + 4 <= ext_end {
        let etype = u16::from_be_bytes([payload[p], payload[p + 1]]);
        let elen = u16::from_be_bytes([payload[p + 2], payload[p + 3]]) as usize;
        let edata = p + 4;
        if edata + elen > payload.len() {
            return None;
        }
        if etype == 0 {
            // server_name: list_len(2) type(1) name_len(2) name
            if elen < 5 {
                return None;
            }
            let name_len = u16::from_be_bytes([payload[edata + 3], payload[edata + 4]]) as usize;
            let name_off = edata + 5;
            if name_off + name_len > payload.len() {
                return None;
            }
            let host = std::str::from_utf8(&payload[name_off..name_off + name_len])
                .ok()?
                .to_string();
            return Some(Sni { host, name_off });
        }
        p = edata + elen;
    }
    None
}

/// Извлечь заголовок Host из HTTP/1.x-запроса.
pub fn extract_http_host(payload: &[u8]) -> Option<String> {
    // Запрос начинается с метода; Host: ищем в заголовках.
    let methods: [&[u8]; 7] = [
        b"GET ",
        b"POST ",
        b"HEAD ",
        b"PUT ",
        b"DELETE ",
        b"OPTIONS ",
        b"CONNECT ",
    ];
    if !methods.iter().any(|m| payload.starts_with(m)) {
        return None;
    }
    let text = std::str::from_utf8(payload).ok()?;
    for line in text.split("\r\n").skip(1) {
        if line.is_empty() {
            break; // конец заголовков
        }
        if let Some(v) = line
            .split_once(':')
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.trim())
        {
            // Host может содержать порт: example.com:8080
            let host = v.split(':').next()?.trim();
            if !host.is_empty() {
                return Some(host.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Минимальный ClientHello с SNI example.com (собран вручную).
    fn client_hello(host: &str) -> Vec<u8> {
        let name = host.as_bytes();
        let mut ext_data = vec![];
        let list_len = (1 + 2 + name.len()) as u16;
        ext_data.extend_from_slice(&list_len.to_be_bytes());
        ext_data.push(0); // host_name
        ext_data.extend_from_slice(&(name.len() as u16).to_be_bytes());
        ext_data.extend_from_slice(name);

        let mut exts = vec![];
        exts.extend_from_slice(&0u16.to_be_bytes()); // type 0
        exts.extend_from_slice(&(ext_data.len() as u16).to_be_bytes());
        exts.extend_from_slice(&ext_data);

        let mut hs_body = vec![0x03, 0x03]; // TLS 1.2
        hs_body.extend_from_slice(&[0u8; 32]); // random
        hs_body.push(0); // session_id len
        hs_body.extend_from_slice(&2u16.to_be_bytes()); // cs len
        hs_body.extend_from_slice(&[0x13, 0x01]); // TLS_AES_128_GCM
        hs_body.push(1); // compression len
        hs_body.push(0);
        hs_body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
        hs_body.extend_from_slice(&exts);

        let mut hs = vec![0x01]; // client_hello
        let l = (hs_body.len() as u32).to_be_bytes();
        hs.extend_from_slice(&l[1..4]);
        hs.extend_from_slice(&hs_body);

        let mut rec = vec![0x16, 0x03, 0x01];
        rec.extend_from_slice(&(hs.len() as u16).to_be_bytes());
        rec.extend_from_slice(&hs);
        rec
    }

    #[test]
    fn sni_extracted_with_offset() {
        let p = client_hello("example.com");
        let s = extract_tls_sni(&p).unwrap();
        assert_eq!(s.host, "example.com");
        assert_eq!(&p[s.name_off..s.name_off + 11], b"example.com");
    }

    #[test]
    fn sni_none_on_garbage() {
        assert!(extract_tls_sni(b"").is_none());
        assert!(extract_tls_sni(b"\x16\x03").is_none());
        assert!(extract_tls_sni(&[0x17, 0x03, 0x03, 0, 10, 0, 0, 0]).is_none());
    }

    #[test]
    fn http_host_extracted() {
        let req = b"GET / HTTP/1.1\r\nHost: example.com:8080\r\nAccept: */*\r\n\r\n";
        assert_eq!(extract_http_host(req).unwrap(), "example.com");
        assert!(extract_http_host(b"GET / HTTP/1.1\r\n\r\n").is_none());
        assert!(extract_http_host(b"BINARYDATA").is_none());
    }
}
