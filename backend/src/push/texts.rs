//! Notification texts in en / zh / ja, rendered on the server in the user's language (contracts C.6).

use serde_json::{json, Value};

/// What the service worker shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub title: String,
    pub body: String,
    /// What the device replaces: one of `remind`, `bank`, `version`, `test`
    pub tag: String,
    pub url: String,
    /// `en`, `zh-CN` or `ja`
    pub lang: &'static str,
}

/// The longest bank name shown (characters).
const NAME_MAX: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Zh,
    Ja,
}

impl Lang {
    fn of(ui_lang: &str) -> Lang {
        let lower = ui_lang.trim().to_ascii_lowercase();
        if lower == "zh" || lower.starts_with("zh-") || lower.starts_with("zh_") {
            Lang::Zh
        } else if lower == "ja" || lower.starts_with("ja-") {
            Lang::Ja
        } else {
            Lang::En
        }
    }

    fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh-CN",
            Lang::Ja => "ja",
        }
    }

    fn pick(self, en: String, zh: String, ja: String) -> String {
        match self {
            Lang::En => en,
            Lang::Zh => zh,
            Lang::Ja => ja,
        }
    }
}

const CHEERS_EN: [&str; 4] = [
    "Keep going!",
    "You are almost there!",
    "A few minutes are enough to continue.",
    "Every question counts!",
];
const CHEERS_ZH: [&str; 4] = [
    "继续加油！",
    "就差一点了！",
    "花几分钟就能接着做。",
    "每一道题都算数！",
];
const CHEERS_JA: [&str; 4] = [
    "その調子です！",
    "あと少しです！",
    "数分あれば続きができます。",
    "一問一問が力になります！",
];

/// A bank name cut to 60 characters, with `…` appended when it was cut.
fn short_name(name: &str) -> String {
    if name.chars().count() <= NAME_MAX {
        return name.to_string();
    }
    let mut cut: String = name.chars().take(NAME_MAX).collect();
    cut.push('…');
    cut
}

fn count(payload: &Value, key: &str) -> i64 {
    payload.get(key).and_then(Value::as_i64).unwrap_or(0).max(0)
}

fn text(payload: &Value, key: &str) -> String {
    short_name(payload.get(key).and_then(Value::as_str).unwrap_or(""))
}

/// The notification of a stored payload; `None` for an unknown kind.
pub fn render(payload: &Value, ui_lang: &str) -> Option<Rendered> {
    let lang = Lang::of(ui_lang);
    let kind = payload.get("k")?.as_str()?;
    let (body, tag, url) = match kind {
        "remind" => {
            let answered = count(payload, "answered");
            let total = count(payload, "total");
            let n = payload.get("n").and_then(Value::as_i64).unwrap_or(1);
            let slot = (n - 1).rem_euclid(4) as usize;
            let exam = payload.get("kind").and_then(Value::as_str) == Some("exam");
            let body = if exam {
                lang.pick(
                    format!(
                        "Your exam is not finished yet ({answered}/{total} answered). {}",
                        CHEERS_EN[slot]
                    ),
                    format!(
                        "你的考试还没做完（已答 {answered}/{total} 题），{}",
                        CHEERS_ZH[slot]
                    ),
                    format!(
                        "試験がまだ終わっていません（{total} 問中 {answered} 問回答済み）。{}",
                        CHEERS_JA[slot]
                    ),
                )
            } else {
                lang.pick(
                    format!(
                        "Your practice is not finished yet ({answered}/{total} answered). {}",
                        CHEERS_EN[slot]
                    ),
                    format!(
                        "你的练习还没做完（已答 {answered}/{total} 题），{}",
                        CHEERS_ZH[slot]
                    ),
                    format!(
                        "練習がまだ終わっていません（{total} 問中 {answered} 問回答済み）。{}",
                        CHEERS_JA[slot]
                    ),
                )
            };
            (body, "remind", "/#/quiz")
        }
        "bank.updated" => {
            let name = text(payload, "name");
            let added = count(payload, "added");
            let total = count(payload, "total");
            (
                lang.pick(
                    format!("The bank \"{name}\" was updated: {added} new, {total} in all."),
                    format!("题库「{name}」已更新：新增 {added} 题，现在共 {total} 题"),
                    format!("問題集「{name}」が更新されました：{added} 問追加、全 {total} 問"),
                ),
                "bank",
                "/#/study",
            )
        }
        "bank.new" => {
            let name = text(payload, "name");
            let total = count(payload, "total");
            (
                lang.pick(
                    format!("You can now use the new bank \"{name}\" ({total} questions)."),
                    format!("你可以使用新题库「{name}」了（共 {total} 题）"),
                    format!("新しい問題集「{name}」が使えるようになりました（全 {total} 問）"),
                ),
                "bank",
                "/#/study",
            )
        }
        "bank.grantMany" => {
            let n = count(payload, "n");
            (
                lang.pick(
                    format!("You can now use {n} more banks."),
                    format!("你又可以使用 {n} 个题库了"),
                    format!("新たに {n} 件の問題集が使えるようになりました"),
                ),
                "bank",
                "/#/study",
            )
        }
        "version" => {
            let from = text(payload, "from");
            let to = text(payload, "to");
            (
                lang.pick(
                    format!("Xexam was updated from v{from} to v{to}. Tap to see what is new."),
                    format!("Xexam 已从 v{from} 更新到 v{to}，点开查看更新内容"),
                    format!(
                        "Xexam が v{from} から v{to} に更新されました。タップして更新内容を確認できます。"
                    ),
                ),
                "version",
                "/#/about",
            )
        }
        "test" => (
            lang.pick(
                "This is a test notification.".to_string(),
                "这是一个测试".to_string(),
                "これはテスト通知です。".to_string(),
            ),
            "test",
            "/#/settings",
        ),
        _ => return None,
    };
    Some(Rendered {
        title: "Xexam".to_string(),
        body,
        tag: tag.to_string(),
        url: url.to_string(),
        lang: lang.code(),
    })
}

/// The plaintext that is encrypted and sent to the push service (D23): compact JSON in the fixed key order
/// `{"v":1,"title","body","tag","url","lang"}`.
pub fn plaintext(r: &Rendered) -> String {
    json!({"v": 1, "title": r.title, "body": r.body, "tag": r.tag, "url": r.url, "lang": r.lang})
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payloads() -> Vec<Value> {
        vec![
            json!({"k":"remind","kind":"practice","answered":3,"total":20,"n":1,"key":"0123456789abcdef0123456789abcdef"}),
            json!({"k":"remind","kind":"exam","answered":12,"total":50,"n":2,"key":"0123456789abcdef0123456789abcdef"}),
            json!({"k":"bank.updated","bank":4,"name":"General knowledge","added":5,"total":60}),
            json!({"k":"bank.new","bank":5,"name":"Kitchen basics","total":30}),
            json!({"k":"bank.grantMany","n":3}),
            json!({"k":"version","from":"3.0","to":"3.1"}),
            json!({"k":"test"}),
        ]
    }

    #[test]
    fn every_key_renders_in_all_languages() {
        for payload in payloads() {
            for (ui, code) in [("en", "en"), ("zh-CN", "zh-CN"), ("ja", "ja")] {
                let r = render(&payload, ui).expect("renders");
                assert_eq!(r.title, "Xexam");
                assert_eq!(r.lang, code);
                assert!(!r.body.is_empty());
                assert!(!r.body.contains('{') && !r.body.contains('}'), "{}", r.body);
                assert!(["remind", "bank", "version", "test"].contains(&r.tag.as_str()));
                assert!(r.url.starts_with("/#/"));
            }
        }
    }

    #[test]
    fn exact_texts() {
        let practice = &payloads()[0];
        assert_eq!(
            render(practice, "en").unwrap().body,
            "Your practice is not finished yet (3/20 answered). Keep going!"
        );
        assert_eq!(
            render(practice, "zh-CN").unwrap().body,
            "你的练习还没做完（已答 3/20 题），继续加油！"
        );
        assert_eq!(
            render(practice, "ja").unwrap().body,
            "練習がまだ終わっていません（20 問中 3 問回答済み）。その調子です！"
        );
        let exam = &payloads()[1];
        assert_eq!(
            render(exam, "en").unwrap().body,
            "Your exam is not finished yet (12/50 answered). You are almost there!"
        );
        assert_eq!(
            render(&payloads()[2], "en").unwrap().body,
            "The bank \"General knowledge\" was updated: 5 new, 60 in all."
        );
        assert_eq!(
            render(&payloads()[3], "zh-CN").unwrap().body,
            "你可以使用新题库「Kitchen basics」了（共 30 题）"
        );
        assert_eq!(
            render(&payloads()[4], "ja").unwrap().body,
            "新たに 3 件の問題集が使えるようになりました"
        );
        assert_eq!(
            render(&payloads()[5], "en").unwrap().body,
            "Xexam was updated from v3.0 to v3.1. Tap to see what is new."
        );
        assert_eq!(
            render(&payloads()[6], "zh-CN").unwrap().body,
            "这是一个测试"
        );
    }

    #[test]
    fn tags_and_urls() {
        let expect = [
            ("remind", "/#/quiz"),
            ("remind", "/#/quiz"),
            ("bank", "/#/study"),
            ("bank", "/#/study"),
            ("bank", "/#/study"),
            ("version", "/#/about"),
            ("test", "/#/settings"),
        ];
        for (payload, (tag, url)) in payloads().iter().zip(expect) {
            let r = render(payload, "en").unwrap();
            assert_eq!((r.tag.as_str(), r.url.as_str()), (tag, url));
        }
    }

    #[test]
    fn cheers_rotate_with_n() {
        let mut seen = Vec::new();
        for n in 1..=5 {
            let p = json!({"k":"remind","kind":"practice","answered":1,"total":2,"n":n});
            seen.push(render(&p, "en").unwrap().body);
        }
        assert!(seen[0].ends_with("Keep going!"));
        assert!(seen[1].ends_with("You are almost there!"));
        assert!(seen[2].ends_with("A few minutes are enough to continue."));
        assert!(seen[3].ends_with("Every question counts!"));
        assert_eq!(seen[4], seen[0]);
        // A missing or odd n never panics
        for n in [json!(0), json!(-7), json!("x"), Value::Null] {
            let p = json!({"k":"remind","kind":"practice","answered":1,"total":2,"n":n});
            assert!(render(&p, "en").is_some());
        }
    }

    #[test]
    fn unknown_kind_and_bad_payload_give_none() {
        assert!(render(&json!({"k":"nope"}), "en").is_none());
        assert!(render(&json!({}), "en").is_none());
        assert!(render(&json!({"k":7}), "en").is_none());
        assert!(render(&Value::Null, "en").is_none());
    }

    #[test]
    fn language_fallbacks() {
        let p = json!({"k":"test"});
        assert_eq!(render(&p, "").unwrap().lang, "en");
        assert_eq!(render(&p, "fr").unwrap().lang, "en");
        assert_eq!(render(&p, "zh").unwrap().lang, "zh-CN");
        assert_eq!(render(&p, "ja-JP").unwrap().lang, "ja");
    }

    #[test]
    fn long_names_are_cut_at_60_characters() {
        let name = "题".repeat(61);
        let p = json!({"k":"bank.new","bank":1,"name":name,"total":9});
        let body = render(&p, "zh-CN").unwrap().body;
        assert!(body.contains(&format!("{}…", "题".repeat(60))));
        let exact = json!({"k":"bank.new","bank":1,"name":"题".repeat(60),"total":9});
        assert!(!render(&exact, "zh-CN").unwrap().body.contains('…'));
    }

    #[test]
    fn worst_case_plaintext_fits_1024_bytes() {
        let name = "題".repeat(100);
        let worst = [
            json!({"k":"remind","kind":"exam","answered":99999,"total":99999,"n":3}),
            json!({"k":"bank.updated","bank":99999,"name":name,"added":99999,"total":99999}),
            json!({"k":"bank.new","bank":99999,"name":name,"total":99999}),
            json!({"k":"bank.grantMany","n":99999}),
            json!({"k":"version","from":"99.99","to":"99.100"}),
            json!({"k":"test"}),
        ];
        for payload in worst {
            for ui in ["en", "zh-CN", "ja"] {
                let r = render(&payload, ui).unwrap();
                let plain = plaintext(&r);
                assert!(plain.len() <= 1024, "{} bytes", plain.len());
            }
        }
    }

    #[test]
    fn plaintext_key_order_and_content() {
        let r = render(&json!({"k":"test"}), "en").unwrap();
        assert_eq!(
            plaintext(&r),
            r#"{"v":1,"title":"Xexam","body":"This is a test notification.","tag":"test","url":"/#/settings","lang":"en"}"#
        );
    }
}
