//! 数据模型。**字段声明顺序 = 旧版 JSON 的键顺序**（serde_json 开了 preserve_order，序列化时按此顺序输出），
//! 不要重排；前端和黄金响应都依赖它。
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 选项（单选 / 多选题）：`{"key": "A", "text": "…"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opt {
    pub key: String,
    pub text: String,
}

/// 热点题的一个小项：`{"text": "…", "choices": ["はい", "いいえ"]}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    pub choices: Vec<String>,
}

/// 中文翻译拆出来的结构。单选 / 多选：`{"stem", "options"}`；热点题：`{"stem", "items"}`。
/// 旧版里两种形状的键不同，所以用两个可选字段、序列化时跳过 None。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zh {
    pub stem: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub items: Option<Vec<String>>,
    /// 热点题：每个小项末尾用**本语言**写的选项（`是 / 否`、`Yes / No`……），个数必须和日语的一致才有。
    /// 没有时（旧文件，或这一列没写选项）界面退回显示日语的选项。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub choices: Option<Vec<Vec<String>>>,
}

/// 英语版拆出来的结构，和中文完全一样（见 [`Zh`]）。
/// 只有这道题的英语能和日语一一对上（选项字母、小项个数都相同）时才有；对不上就没有 `en`，页面把「英语」按钮置灰。
pub type En = Zh;

/// 题型。序列化成 `"single" | "multi" | "hotspot"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Single,
    Multi,
    Hotspot,
}

/// 答案：单选 / 多选是字母（`["B"]`），热点题是每个小项选了第几个（`[0, 1, 0]`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Answer {
    Letters(Vec<String>),
    Indexes(Vec<i64>),
}

/// 解析器产出的一道题（存进 `questions.data` 的是去掉 id / sheet / row 之后的部分）。
///
/// 对外（`/api/bank`）的键顺序：kind, tag, stem, options, items, answer, translation, zh, [english, en,]
/// explanation, mnemonic, note, id, sheet, row（方括号里的两个只在有英语时出现） —— 与 `questions.data` 的键顺序加上末尾追加的 id、sheet、row 一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub kind: Kind,
    pub tag: String,
    pub stem: String,
    pub options: Vec<Opt>,
    pub items: Vec<Item>,
    pub answer: Answer,
    pub translation: String,
    pub zh: Option<Zh>,
    /// 英语原文（4 列格式的 A 列）。没有英语时为空，且不出现在 JSON 里，所以旧题的输出与以前完全一样。
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub english: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub en: Option<En>,
    pub explanation: String,
    pub mnemonic: String,
    pub note: String,
    // —— 以下三个只在内存里 / 对外输出里有；存库时不写进 data ——
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sheet: String,
    #[serde(default)]
    pub row: i64,
}

/// 某个工作表在解析时的统计。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRow {
    pub row: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSheet {
    pub name: String,
    pub total: i64,
    pub usable: i64,
    pub skipped: Vec<SkippedRow>,
}

/// 整份工作簿的解析结果。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedWorkbook {
    pub sheets: Vec<ParsedSheet>,
    pub questions: Vec<Question>,
}

/// 一条导入记录（`imports` 表的一行，也是 `/api/imports` 的元素）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub id: i64,
    pub at: String,
    pub filename: String,
    pub added: i64,
    pub updated: i64,
    pub removed: i64,
    pub restored: i64,
    pub skipped: i64,
    pub total: i64,
}

/// `/api/attempt` 清洗之后的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptResult {
    pub id: String,
    pub correct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub results: Vec<AttemptResult>,
    pub sheets: Vec<String>,
    pub mode: String,
    pub seconds: i64,
}

/// 预览 / 确认结果里"每个工作表"那一块。键顺序见旧版黄金响应 #114 / #120。
pub type JsonValue = Value;
