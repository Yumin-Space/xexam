//! Rule 3: verdicts on hotspot statements.

use super::util::*;

fn fx() -> (
    xexam_ai_service::study::model::StudyQuestion,
    xexam_ai_service::study::model::Solution,
) {
    hotspot()
}

#[test]
fn tight_verdicts_in_english() {
    let fx = fx();
    for t in [
        "Statement 1 is true.",
        "statement 2 is false",
        "Item 2: no",
        "item 2 - yes",
        "item 3 are wrong",
        "The first statement is false",
        "the second item is wrong",
        "the third one: yes",
        "#1 is correct",
        "#2 incorrect",
        "(2) no",
        "[3] yes",
        "Statement 1 should be true",
        "Statement 1 the answer is yes",
        "Statement 1 → yes",
        "Statement 1 = right",
        "no, statement 1",
        "False: statement 2",
        "Statement 1\nis true",
    ] {
        assert!(whole(&fx, t), "{t:?}");
    }
}

#[test]
fn line_initial_numbers() {
    let fx = fx();
    for t in [
        "Look at these.\n1. false\n2. true",
        "Look.\n2) yes",
        "Look.\n3: no",
        "Look.\n1、はい",
        "Look.\n1：no",
        "Look.\n\n- 2. yes",
    ] {
        assert!(whole(&fx, t), "{t:?}");
    }
    // A number that does not start a line is not a reference.
    assert!(!whole(&fx, "We saw 1. yes was not mentioned"));
}

#[test]
fn zh_and_ja_verdicts() {
    let fx = fx();
    for t in [
        "第1项选是",
        "第1项：是。",
        "第1项是对的",
        "第2条错误",
        "第3句不对",
        "第1个陈述成立",
        "第二项为假",
        "第2个说法不成立",
        "第1项为真",
        "第1项是错的",
        "第2项选否",
        "第3项：错",
        "第1项对。",
        "第1项，是，",
        "第一项应该选是",
        "第2项答是",
        "第3项填否。",
        "1番目はいいえ",
        "2番目の文は正しい",
        "3番目の記述は誤り",
        "項目1は間違い",
        "記述2は正解",
        "二番目の項目は不正解",
        "1番目の文ははい",
    ] {
        assert!(whole(&fx, t), "{t:?}");
    }
}

#[test]
fn single_cjk_claim_words_need_a_clause_position() {
    let fx = fx();
    assert!(!whole(&fx, "第1项是关于服务器状态的描述"));
    assert!(!whole(&fx, "第2项对于备份很重要"));
    assert!(!whole(&fx, "第3项错综复杂"));
    assert!(whole(&fx, "第1项选是"));
    assert!(whole(&fx, "第1项：是。"));
    // A line end after the word is a clause end.
    assert!(whole(&fx, "第1项是\n下一行"));
    assert!(whole(&fx, "第1项否"));
}

#[test]
fn loose_choice_text_near_a_reference() {
    let fx = fx();
    assert!(whole(&fx, "Statement 3 involves the web-node"));
    assert!(whole(&fx, "The db-node handles statement 3"));
    // A sentence end between them breaks the link; so does a long gap.
    assert!(!whole(&fx, "Statement 3 is odd. The web-node is separate"));
    assert!(!whole(
        &fx,
        "Statement 3 is a statement about something far away from web-node"
    ));
}

#[test]
fn neutral_statements_are_released() {
    let fx = fx();
    for t in [
        "Statement 1 describes whether the server is online.",
        "Think about statement 2 and how backups are scheduled.",
        "Statement 1 is about uptime. Yes, that matters in general.",
        "Read each statement slowly before you decide.",
        "Look at item 3 and what serves traffic in a cluster.",
        "Consider what a load balancer does.",
    ] {
        assert!(!whole(&fx, t), "{t:?}");
    }
}

#[test]
fn references_beyond_the_item_count_are_ignored() {
    let fx = fx();
    assert!(!whole(&fx, "Statement 4 is true"));
    assert!(!whole(&fx, "Item 10 is false"));
    assert!(!whole(&fx, "Statement 12 is true"));
    assert!(!whole(&fx, "第9项选是"));
    assert!(!whole(&fx, "the fourth statement is false"));
    assert!(!whole(&fx, "Statement 0 is true"));
}

#[test]
fn tight_gap_is_bounded() {
    let fx = fx();
    // More than 8 punctuation / space characters between them.
    assert!(!whole(&fx, "Statement 1 ::::::::::: true"));
    assert!(whole(&fx, "Statement 1 :::: true"));
    // Other words are not connectors.
    assert!(!whole(&fx, "Statement 1 depends on whether this is true"));
}

#[test]
fn hotspot_key_text_quotes_apply() {
    let q = hotspot().0;
    let fx = (q, sol(&["Yes", "No", "The server is online and healthy"]));
    assert!(whole(
        &fx,
        "Remember that the server is online and healthy matters."
    ));
    assert!(!whole(&fx, "Remember uptime matters."));
}

#[test]
fn rule_one_does_not_apply_to_hotspots() {
    let fx = fx();
    assert!(!whole(&fx, "The answer is B"));
    assert!(!whole(&fx, "Choose option 2"));
}

#[test]
fn tight_gap_edges_in_characters_and_punctuation() {
    let fx = fx();
    // Connector words are free; the whole gap is at most 40 characters.
    let gap40 = "应该".repeat(20);
    assert_eq!(gap40.chars().count(), 40);
    assert!(whole(&fx, &format!("Statement 1{gap40}true")));
    assert!(!whole(&fx, &format!("Statement 1{gap40}是true")));
    assert!(!whole(
        &fx,
        &format!("Statement 1{}true", "应该".repeat(21))
    ));
    // At most 8 punctuation / space / symbol characters.
    assert!(whole(&fx, &format!("Statement 1{}true", ":".repeat(8))));
    assert!(!whole(&fx, &format!("Statement 1{}true", ":".repeat(9))));
    // Spaces count as punctuation: 8 spaces pass, 9 do not.
    assert!(whole(&fx, &format!("Statement 1{}true", " ".repeat(8))));
}

#[test]
fn a_connector_word_inside_a_longer_innocent_sentence_is_no_claim() {
    let fx = fx();
    for t in [
        "Statement 1 is something you should reason about before you decide whether it is clear.",
        "Statement 2 is the answer to a different kind of question, so read it carefully.",
        "Item 3 is where you choose a tool, and you select it after reading.",
        "第1项是关于备份策略的说明，应该仔细阅读。",
    ] {
        assert!(!whole(&fx, t), "{t:?}");
    }
}

#[test]
fn claim_word_before_the_reference_with_a_preposition() {
    let fx = fx();
    // Spec 5.11 positive.
    assert!(whole(&fx, "Yes for statement 3"));
    assert!(whole(&fx, "No for statement 3"));
    for t in [
        "yes on item 2",
        "True about statement 1",
        "false re statement 2",
        "是，对于第3项",
        "错，关于第2项",
        "はい、3番目について",
        "いいえ、2番目に関して",
        "はい、1番目に対して",
        "第3项关于是",
        "3番目については、はい",
    ] {
        assert!(whole(&fx, t), "{t:?}");
    }
    // A longer word that merely starts with a connector is no connector.
    assert!(!whole(&fx, "Yes forgetting statement 3"));
    assert!(!whole(&fx, "Statement 1 onward true"));
}

#[test]
fn hedged_and_negated_verdicts_are_caught() {
    let fx = fx();
    for t in [
        "Statement 2 is not true",
        "Statement 2 is actually false",
        "Statement 1 is definitely true",
        "Statement 1 should be answered yes",
        "Statement 1 holds, so choose yes",
        "Statement two is true",
        "Statement 2 isn't true",
        "Item three is certainly false",
        "In statement 3 the db node",
        "Statement 1 is also marked yes",
    ] {
        assert!(whole(&fx, t), "{t:?}");
    }
    // Spelled-out numbers beyond the item count are ignored.
    assert!(!whole(&fx, "Statement four is true"));
    assert!(!whole(&fx, "Statement ten is true"));
}

#[test]
fn questions_about_a_statement_are_not_verdicts() {
    let fx = fx();
    for t in [
        "Decide whether statement 2 is true or false by reading the logs.",
        "Look at statement 2, no need to rush.",
        "Decide if item 3 is correct by checking the nodes.",
        "Ask yourself whether item 1 is true",
        "Is statement 2 true or false? Check the logs.",
        "Statement 1 is true or false depending on the status page.",
        "关于第1项，请判断是否为真",
        "第2项是否正确，请自己判断",
        "1番目の文が正しいかどうか考えましょう",
    ] {
        assert!(!whole(&fx, t), "{t:?}");
    }
    // A comma or sentence end separates a question word from the verdict.
    assert!(whole(&fx, "If you ask me, statement 1 is true"));
    assert!(whole(&fx, "Whether or not you agree: statement 1 is true"));
    assert!(whole(&fx, "Decide now. Statement 1 is true"));
    // Plain `no` as a verdict still counts.
    assert!(whole(&fx, "Statement 1: no"));
    assert!(whole(&fx, "Statement 1 - no."));
}
