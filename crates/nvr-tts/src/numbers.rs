//! Arabic numerals to Chinese readings for the lexicon front end, which only
//! knows characters: `22` → 二十二, `3.14` → 三点一四, `13800138000` →
//! 一三八零零一三八零零零.

const DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
/// Numbers with more digits are read digit by digit (phone numbers, IDs).
const MAX_CARDINAL_DIGITS: usize = 8;

/// Replace every run of ASCII digits (with an optional decimal part) by its
/// Chinese reading; everything else is kept.
pub(crate) fn normalize(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let int: String = chars[start..i].iter().collect();
        out.push_str(&integer(&int));
        if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
            i += 1;
            out.push('点');
            while i < chars.len() && chars[i].is_ascii_digit() {
                out.push(digit(chars[i]));
                i += 1;
            }
        }
    }
    out
}

fn digit(c: char) -> char {
    DIGITS[c.to_digit(10).unwrap_or(0) as usize]
}

fn integer(digits: &str) -> String {
    if digits.len() > MAX_CARDINAL_DIGITS || (digits.len() > 1 && digits.starts_with('0')) {
        return digits.chars().map(digit).collect();
    }
    // At most 8 digits, so it fits.
    let n: u32 = digits.parse().unwrap_or(0);
    cardinal(n)
}

/// Reading of `n` < 10^8, e.g. 10010 → 一万零一十, 12 → 十二.
fn cardinal(n: u32) -> String {
    if n == 0 {
        return "零".into();
    }
    let (high, low) = (n / 10_000, n % 10_000);
    let mut out = String::new();
    if high > 0 {
        out.push_str(&below_10000(high));
        out.push('万');
        if low > 0 && low < 1000 {
            out.push('零');
        }
    }
    if low > 0 {
        out.push_str(&below_10000(low));
    }
    // 一十二 is read 十二 at the start of a number.
    if let Some(rest) = out.strip_prefix("一十") {
        out = format!("十{rest}");
    }
    out
}

fn below_10000(n: u32) -> String {
    const UNITS: [&str; 4] = ["千", "百", "十", ""];
    let digits = [n / 1000, n / 100 % 10, n / 10 % 10, n % 10];
    let mut out = String::new();
    let mut zero = false;
    for (d, unit) in digits.iter().zip(UNITS) {
        if *d == 0 {
            zero = !out.is_empty();
            continue;
        }
        if zero {
            out.push('零');
            zero = false;
        }
        out.push(DIGITS[*d as usize]);
        out.push_str(unit);
    }
    out
}

#[cfg(test)]
#[path = "numbers_test.rs"]
mod numbers_test;
