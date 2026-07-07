pub fn wrap_line_count(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    if line.trim().is_empty() {
        return 1;
    }
    let mut count = 0usize;
    let mut cur = 0usize;
    for word in line.split_whitespace() {
        let wlen = word.chars().count();
        if wlen > width {
            if cur > 0 {
                count += 1;
            }
            count += wlen / width;
            cur = wlen % width;
            continue;
        }
        if cur == 0 {
            cur = wlen;
        } else if cur + 1 + wlen <= width {
            cur += 1 + wlen;
        } else {
            count += 1;
            cur = wlen;
        }
    }
    if cur > 0 {
        count += 1;
    }
    count
}

pub fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    if line.trim().is_empty() {
        return vec![String::new()];
    }
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for word in line.split_whitespace() {
        let wlen = word.chars().count();
        if wlen > width {
            if !cur.is_empty() {
                parts.push(cur);
                cur = String::new();
                cur_len = 0;
            }
            let mut chars = word.chars().peekable();
            while chars.peek().is_some() {
                let chunk: String = chars.by_ref().take(width).collect();
                let chunk_len = chunk.chars().count();
                if chunk_len == width {
                    parts.push(chunk);
                } else {
                    cur = chunk;
                    cur_len = chunk_len;
                }
            }
            continue;
        }
        if cur.is_empty() {
            cur.push_str(word);
            cur_len = wlen;
        } else if cur_len + 1 + wlen <= width {
            cur.push(' ');
            cur.push_str(word);
            cur_len += 1 + wlen;
        } else {
            parts.push(cur);
            cur = word.to_string();
            cur_len = wlen;
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}
