//! Rule 1 and Rule 2 on whole texts: en / zh / ja option claims and quotes.

use super::util::*;

fn leak(
    fx: &(
        xexam_ai_service::study::model::StudyQuestion,
        xexam_ai_service::study::model::Solution,
    ),
    t: &str,
) {
    assert!(whole(fx, t), "expected a leak: {t:?}");
}

fn clean(
    fx: &(
        xexam_ai_service::study::model::StudyQuestion,
        xexam_ai_service::study::model::Solution,
    ),
    t: &str,
) {
    assert!(!whole(fx, t), "expected no leak: {t:?}");
}

#[test]
fn neutral_hints_are_released() {
    let fx = single();
    for t in [
        "Think about orbital distance and how much sunlight each body receives.",
        "Compare the orbital periods; shorter orbits belong to bodies nearer the star.",
        "The innermost world has no atmosphere to speak of.",
        "A good way to start is to list what you know about the inner solar system.",
        "Recall the order of the planets from the sun.",
        "Kepler's third law links distance and period.",
        "思考一下轨道距离与阳光强度的关系。",
        "各天体の公転周期を比べてみましょう。",
    ] {
        clean(&fx, t);
    }
}

#[test]
fn english_lead_then_letter() {
    let fx = single();
    for t in [
        "The answer is B.",
        "the answer is b",
        "The correct option is c",
        "I would pick D here",
        "Go with D",
        "You can rule out C.",
        "Eliminate B and C",
        "Best: A. Others are weaker",
        "choose a)",
        "pick a.",
        "select a:",
        "Option\tB",
    ] {
        leak(&fx, t);
    }
}

#[test]
fn lowercase_a_needs_a_closing_mark() {
    let fx = single();
    clean(&fx, "Choose a simple method and try it.");
    clean(&fx, "The answer is a matter of distance.");
}

#[test]
fn lowercase_letters_after_a_lead() {
    let fx = single();
    for (t, expect) in [
        ("choose b", true),
        ("choose c now", true),
        ("pick d", true),
        ("the answer is e", false), // E is not a key of a four-option question
        ("pick bd", false),         // not standalone
    ] {
        assert_eq!(whole(&fx, t), expect, "{t:?}");
    }
}

#[test]
fn letter_gap_is_at_most_forty_without_a_sentence_end() {
    let fx = single();
    leak(&fx, "answer: the one marked c");
    clean(&fx, "answer. Then consider that the one marked c is odd");
    // 45 filler characters between the lead and the lowercase letter.
    clean(
        &fx,
        "answer 123456789012345678901234567890123456789012345 c",
    );
    leak(&fx, "answer 12345678901234567890123456789012345 c");
}

#[test]
fn plain_word_verdicts_are_withheld() {
    let fx = single();
    for t in [
        "In short, A is the right answer.",
        "The first option is the right one.",
        "A是正确的。",
        "第一个选项是正确的。",
        "The correct answer, as you might expect, is A.",
        "I'd go for A.",
        "A is the right choice here.",
        "A is the one to pick.",
        "A is your answer.",
        "You should lean toward A.",
        "The first option is the right answer.",
        "The first and third options are correct.",
        "The first and third choices are the right ones.",
        "A项正确。",
        "A才是正确答案。",
        "第一项是正确答案。",
        "第一和第三个选项是正确的。",
    ] {
        leak(&fx, t);
    }
}

#[test]
fn hotspot_plain_word_verdicts_are_withheld() {
    let items = (0..3)
        .map(|i| xexam_ai_service::study::model::HotspotItem {
            text: format!("Statement text number {i}"),
            choices: vec!["Yes".to_string(), "No".to_string()],
        })
        .collect();
    let fx = (
        question_full(Kind::Hotspot, "Review the statements.", Vec::new(), items),
        sol(&["Yes", "No"]),
    );
    for t in [
        "Statement 1 is accurate, statement 2 is not.",
        "第一条说法是对的。",
    ] {
        leak(&fx, t);
    }
}

#[test]
fn plain_word_verdicts_in_a_stem_with_a_lone_letter() {
    let fx = single_class_b();
    leak(&fx, "B is the right answer.");
}

#[test]
fn english_trailing_verdicts() {
    let fx = single();
    for t in [
        "B is correct.",
        "B is right",
        "(B) is the answer",
        "B is the correct choice",
        "B is the best fit",
        "C is wrong",
        "C is incorrect",
        "D is not it",
        "D isn't it",
        "D isn\u{2019}t it",
        "B is false",
        "B is true",
        "C is a distractor",
        "C can be ruled out",
        "C can be eliminated",
        "D should be chosen",
        "Mars is wrong",
        "Mercury can be ruled out",
        "\"B\" is correct",
    ] {
        leak(&fx, t);
    }
}

#[test]
fn trail_punct_gap_is_at_most_six() {
    let fx = single();
    leak(&fx, "Mars ** - is wrong");
    clean(&fx, "Mars ******* is wrong");
}

#[test]
fn ordinals_in_english() {
    let fx = single();
    for t in [
        "Go with the second option",
        "Pick the first answer",
        "I'd choose the third one",
        "The fourth option is wrong",
        "Choose the last choice",
        "The second option is correct",
        "rule out the last one",
        "Choose option 2",
        "choice 3 is wrong",
        "Pick #4",
        "pick the second of those options",
    ] {
        let expect = !t.contains("those");
        assert_eq!(whole(&fx, t), expect, "{t:?}");
    }
    // An ordinal past the option count is no reference.
    clean(&fx, "Go with the fifth option");
    clean(&fx, "Choose option 9");
    clean(&fx, "Choose #5");
}

#[test]
fn ordinal_words_need_a_noun_within_two_words() {
    let fx = single();
    clean(&fx, "Choose the first step carefully");
    leak(&fx, "choose the first real answer");
    clean(&fx, "choose the first of the many answers");
    clean(&fx, "choose the second of the many remaining planets");
}

#[test]
fn zh_claims() {
    let fx = single();
    for t in [
        "答案是B",
        "答案是 b",
        "应选 c",
        "应该选D",
        "正确的是B",
        "排除C",
        "选择第二个选项",
        "我会选第1项",
        "第3项是错的",
        "第二个答案不正确",
        "最后一个选项不对",
        "最后一个不是",
        "第四项错误",
        "第2个选项正确",
        "选项B错误",
        "水星不是",
        "水星可以排除",
        "水星不符合",
        "水星可排除",
        "B 才是答案",
        "B是答案",
    ] {
        leak(&fx, t);
    }
    clean(&fx, "第五项是什么？");
}

#[test]
fn ja_claims() {
    let fx = single();
    for t in [
        "正解はBです",
        "答えはc",
        "選択肢Bを除外",
        "Dを選んで",
        "Bを選ぶ",
        "Marsは間違い",
        "Marsは誤り",
        "Marsが正しい",
        "Marsが正解",
        "Marsは正解",
        "Marsは正しい",
        "Marsを選",
        "Marsではない",
        "Marsは違う",
        "Marsを除外",
        "Marsが適切",
        "Marsは不適切",
        "2番目が正解",
        "三番目は間違い",
        "最初の選択肢は正しい",
        "最後のものを選んで",
        "最初のものは違う",
        "最後の選択肢を除外",
        "水星は間違い",
    ] {
        leak(&fx, t);
    }
}

#[test]
fn full_width_forms_are_normalised() {
    let fx = single();
    leak(&fx, "答案是Ｂ");
    leak(&fx, "（Ｂ）");
    leak(&fx, "Ｂ）");
    leak(&fx, "Ａ：");
}

#[test]
fn option_texts_of_any_length_after_a_lead_or_before_a_trail() {
    let long =
        "a very long distractor option text that goes on and on about nothing in particular at all";
    let q = question(Kind::Single, "Which one?", &["Short", "Y", long, "Last"]);
    let fx = (q, sol(&["Short"]));
    leak(&fx, "I would choose Y");
    leak(&fx, "Y is wrong");
    leak(&fx, &format!("The answer is {long}"));
    leak(&fx, &format!("{long} is not it"));
    clean(&fx, "Think about why Y is a letter.");
    // 120-char limit: a longer option text is not a checked text.
    let very_long = "x".repeat(10) + &" word".repeat(25);
    assert!(very_long.chars().count() > 120);
    let q2 = question(Kind::Single, "Which one?", &["Short", &very_long]);
    let fx2 = (q2, sol(&["Short"]));
    clean(&fx2, &format!("choose {very_long}"));
    // 120 characters exactly is still checked.
    let exact = "x".repeat(120);
    let q3 = question(Kind::Single, "Which one?", &["Short", &exact]);
    let fx3 = (q3, sol(&["Short"]));
    leak(&fx3, &format!("choose {exact}"));
}

#[test]
fn x_texts_are_word_bounded_for_latin() {
    let fx = single();
    clean(&fx, "Choose Marshmallow, not a planet.");
    leak(&fx, "choose mars");
    leak(&fx, "CHOOSE MARS");
}

#[test]
fn x_texts_in_cjk_are_plain_substrings() {
    let q = question(
        Kind::Single,
        "哪一个最近？",
        &["水星", "金星", "地球", "火星"],
    );
    let fx = (q, sol(&["水星"]));
    leak(&fx, "应选水星");
    leak(&fx, "金星不是");
    leak(&fx, "選ぶなら地球");
}

#[test]
fn key_text_quotes_rule_two() {
    let fx = single();
    // 12+ non-space characters: quoted anywhere, with no lead at all.
    leak(&fx, "Hmm, the nearest planet to the sun is what we want.");
    leak(&fx, "THE NEAREST   PLANET TO THE SUN");
    // Shorter key texts: Rule 2b (see `bare_short_key_text_is_a_leak`).
    leak(&fx, "Mercury has no moons.");
    // CJK key text with 5 or more characters.
    let q = question(Kind::Single, "哪一颗行星离太阳最近？", &["水星", "金星"]);
    let fx2 = (q, sol(&["离太阳最近的行星", "水星"]));
    leak(&fx2, "想一想离太阳最近的行星是什么");
    // 4 CJK characters are below the quote floor but Rule 2b catches them.
    let fx3 = (
        question(Kind::Single, "哪个？", &["水星", "金星"]),
        sol(&["水星一号", "水星"]),
    );
    leak(&fx3, "水星一号听起来像火箭");
}

#[test]
fn bare_short_key_text_is_a_leak() {
    let fx = single();
    for t in [
        "It is Mercury",
        "The closest planet is Mercury",
        "Think of the messenger god, Mercury.",
        "the planet nearest the sun? That is mercury!",
        "水星最近",
    ] {
        leak(&fx, t);
    }
    // Not word-bounded: a longer word is no mention.
    clean(&fx, "Mercurial moods are a different topic.");
    // A key text that the stem or a wrong option also contains is not checked.
    let shared = (
        question(
            Kind::Single,
            "Which port does the web service use?",
            &["port 443", "port 8443", "port 80", "port 22"],
        ),
        sol(&["port 443"]),
    );
    clean(&shared, "Think about secure web traffic and its port.");
    let in_stem = (
        question(
            Kind::Single,
            "Is RAID 5 a parity scheme or a mirror?",
            &["RAID 5 parity", "a mirror", "striping", "none"],
        ),
        sol(&["RAID 5 parity"]),
    );
    clean(&in_stem, "Think about how RAID 5 is built.");
    // Naming a WRONG option is released; yes / no style keys are left to the
    // verdict rules.
    clean(&fx, "Venus is hotter than you might think.");
    let tf = (
        question(
            Kind::Single,
            "Light travels faster than sound.",
            &["True", "False"],
        ),
        sol(&["True", "对"]),
    );
    clean(
        &tf,
        "Compare the two speeds and decide whether that is true in general.",
    );
    // Hotspots use rule 3 only.
    let hs = hotspot();
    clean(&hs, "Remember web nodes and database roles.");
    // Short key texts (below 3 characters) are left to Rules 1 and 3.
    let tiny = (
        question(Kind::Single, "Pick the prime.", &["4", "7", "9", "8"]),
        sol(&["7"]),
    );
    clean(&tiny, "Seven is a small number: 7 appears here.");
}

#[test]
fn key_text_quote_for_multi_and_other_languages() {
    let fx = multi();
    clean(&fx, "Think about density and composition.");
    let fx = (fx.0, sol(&["Mercury", "Earth", "金星と地球は岩石惑星です"]));
    leak(&fx, "金星と地球は岩石惑星です、と覚えましょう");
}

#[test]
fn bare_letters_b_to_h() {
    let fx = single();
    for t in [
        "Compare B with C",
        "Look at D.",
        "Between C and D, hmm",
        "思考 B 的轨道",
    ] {
        leak(&fx, t);
    }
    // Letters that are not option keys of a four-option question.
    clean(&fx, "Think about Vitamin E and plan F");
    // Bare A is not flagged.
    clean(&fx, "A planet orbits a star.");
}

#[test]
fn bare_letter_exempt_when_standing_alone_in_the_stem() {
    let fx = single_class_b();
    clean(
        &fx,
        "A Class B observatory has modest optics, so think about Class B stars.",
    );
    clean(&fx, "Remember what Class B means here.");
    // Rules 1a / 1b still apply to that letter.
    leak(&fx, "The answer is B");
    leak(&fx, "B is correct");
    leak(&fx, "Pick B");
    // Other letters stay checked bare.
    leak(&fx, "Compare with C");
}

#[test]
fn bare_letter_exempt_when_standing_alone_in_an_option_text() {
    let q = question(
        Kind::Single,
        "Pick a grade.",
        &["Grade A", "Grade B", "Grade C", "Grade D"],
    );
    let fx = (q, sol(&["Grade A"]));
    clean(&fx, "Consider how level C differs in general terms.");
    clean(&fx, "Think about C and D in the abstract.");
    leak(&fx, "The answer is C");
}

#[test]
fn marked_forms_of_any_key_letter() {
    let fx = single();
    for t in [
        "see **A** for details",
        "*A* is interesting",
        "look at (A)",
        "look at （A）",
        "see [A]",
        "A) first",
        "A） first",
        "the \"A\" one",
        "the “A” one",
        "the 「A」 one",
        "A. Something",
        "Line one\nA. Something",
        "Line one\n\nA: Something",
        "Line one\nA： Something",
        "Line one\nA、 Something",
        "intro\n- A. thing",
    ] {
        leak(&fx, t);
    }
}

#[test]
fn marked_forms_are_not_exempt_by_the_stem() {
    let fx = single_class_b();
    leak(&fx, "look at (B)");
}

#[test]
fn bare_a_in_prose_is_fine() {
    let fx = single();
    clean(&fx, "A key idea is distance.");
    clean(&fx, "Use a ruler. Then a calculator.");
    clean(&fx, "Choose a method, then apply it.");
    clean(&fx, "There is a) no way to be sure");
}

#[test]
fn multi_questions_use_the_same_rules() {
    let fx = multi();
    leak(&fx, "Pick A and C");
    leak(&fx, "Neptune is wrong");
    leak(&fx, "Gas giant is not rocky");
    clean(&fx, "Rocky planets have solid surfaces.");
}

#[test]
fn sentence_end_stops_a_lead() {
    let fx = single();
    clean(&fx, "Consider the options. Venus has a thick atmosphere");
    leak(&fx, "Consider the options: Venus");
    clean(&fx, "Consider the options.\n\nVenus is hot");
    // A blank line is a sentence end, a single line break is not.
    leak(&fx, "Consider the options\nVenus");
}

#[test]
fn letter_followed_by_an_option_noun_is_checked_even_when_exempt() {
    let fx = single_class_b();
    for t in [
        "B option",
        "B 选项",
        "B選択肢",
        "the B choice",
        "B answer",
        "C 答案",
        "A 选项",
    ] {
        leak(&fx, t);
    }
    // `A` before an English noun is an article.
    clean(&fx, "A choice of words matters; a option is not a thing.");
    clean(&fx, "Class B stars are hot.");
}

#[test]
fn contractions_and_abbreviations_are_no_option_letters() {
    let fx = single();
    for t in [
        "The best thing you'd do is compare orbits.",
        "Remember, you'd pick what you'd know.",
        "I'd say, he'd agree, it'd help to compare orbits.",
        "The best way, e.g. looking at orbits, is slow.",
        "Pick a method, i.e. a plan, before you start.",
    ] {
        clean(&fx, t);
    }
    // A five-option question has the letter e as an option key.
    let five = (
        question(
            Kind::Single,
            "Which planet is closest to the sun?",
            &["Mercury", "Venus", "Earth", "Mars", "Saturn"],
        ),
        sol(&["Mercury"]),
    );
    clean(&five, "The best way, e.g. looking at orbits, is slow.");
    clean(&five, "Pick a plan, i.e. a method, first.");
    // Real letters are still caught.
    leak(&five, "The answer is e");
    leak(&fx, "I'd pick d");
    leak(&fx, "The answer is d.");
}
