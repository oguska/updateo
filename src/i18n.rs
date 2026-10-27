//! Arayüz dili: `LC_ALL` → `LC_MESSAGES` → `LANG` sırasıyla ilk boş olmayan
//! değere karar verilir; `tr` öneki Türkçe, diğer her şey (C/POSIX dahil ve
//! boşsa) İngilizce yapar.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Tr,
    En,
}

static LANG: OnceLock<Lang> = OnceLock::new();

pub fn lang() -> Lang {
    *LANG.get_or_init(|| {
        let vals: Vec<String> = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .filter(|v| !v.is_empty())
            .collect();
        detect(&vals)
    })
}

/// `vals` öncelik sıralı gelir; ilk geçerli değer belirler.
fn detect(vals: &[String]) -> Lang {
    vals.iter()
        .map(|v| v.to_lowercase())
        .find(|v| v != "c" && v != "posix")
        .map(|v| {
            if v.starts_with("tr") {
                Lang::Tr
            } else {
                Lang::En
            }
        })
        .unwrap_or(Lang::En)
}

/// Kullanıcıya görünen metni diline göre seç.
pub fn tr(en: &str, tr: &str) -> String {
    if lang() == Lang::Tr {
        tr.to_string()
    } else {
        en.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn detection() {
        assert_eq!(detect(&v(&["tr_TR.UTF-8"])), Lang::Tr);
        assert_eq!(detect(&v(&["en_US.UTF-8"])), Lang::En);
        // öncelik: boş olmayan ilk değer kazanır
        assert_eq!(detect(&v(&["", "tr_TR"])), Lang::En); // "" filtrelenmiş sayılmazsa ilk ""
        assert_eq!(detect(&v(&["C"])), Lang::En);
        assert_eq!(detect(&v(&[])), Lang::En);
        assert_eq!(
            detect(&v(&["TR_tr.UTF-8".to_lowercase().as_str()])),
            Lang::Tr
        );
    }
}
