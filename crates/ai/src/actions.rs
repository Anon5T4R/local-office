//! The document actions: request -> messages, answer -> [`Outcome`].
//!
//! The window applies an outcome through the engine's own commands; the model never writes a
//! `.docx` itself. The answer is read by the shared tolerant extractor ([`local_ai::extract`]), so
//! reasoning blocks, fences and prose around the JSON do not have to be handled here.
//!
//! Destructivity zero: `GenerateDoc` builds a **new** document, and `EditDoc` only ever applies to
//! a copy the window puts beside the original (`<name> AI.docx`) - the file the user opened is
//! never written by the AI.

use local_ai::Msg;
use serde::Deserialize;

use crate::prompts::{fill, for_lang};

/// One block of a document, in the closed vocabulary the model may use.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph { text: String },
    List { items: Vec<String> },
}

/// What the user asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// A free description -> a whole new document, in `language`. `shape` is the dialog's preset
    /// ("a report", "a letter"), appended to the request; `None` asks for nothing.
    GenerateDoc { topic: String, language: String, shape: Option<String> },
    /// The current document's text plus an instruction -> the whole rewritten document.
    EditDoc { instruction: String, document: String },
}

/// What the answer became, ready to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Doc { title: String, blocks: Vec<Block> },
}

/// How many blocks one answer may carry (a runaway answer must not hang the window).
pub const MAX_BLOCKS: usize = 400;

impl Request {
    /// The short label the status strip shows while the worker runs.
    pub fn label(&self) -> &'static str {
        match self {
            Request::GenerateDoc { .. } => "Writing the document",
            Request::EditDoc { .. } => "Rewriting the document",
        }
    }

    /// The subject appended to the label.
    pub fn subject(&self) -> &str {
        match self {
            Request::GenerateDoc { topic, .. } => topic,
            Request::EditDoc { .. } => "this document",
        }
    }
}

/// The conversation one request sends: a system turn and one user turn.
pub fn messages(request: &Request, lang: &str) -> Result<Vec<Msg>, String> {
    let prompts = for_lang(lang);
    let system = Msg::System(fill(prompts.system, &[("answer_in", prompts.answer_in)]));
    let user = match request {
        Request::GenerateDoc { topic, language, shape } => {
            let topic = topic.trim();
            if topic.is_empty() {
                return Err("the topic is empty".into());
            }
            let mut user = fill(prompts.generate, &[("topic", topic), ("language", language), ("answer_in", prompts.answer_in)]);
            if let Some(shape) = shape.as_deref().map(str::trim).filter(|shape| !shape.is_empty()) {
                user.push_str(&format!("\n\nThe document's shape: {shape}."));
            }
            user
        }
        Request::EditDoc { instruction, document } => {
            let instruction = instruction.trim();
            if instruction.is_empty() {
                return Err("the instruction is empty".into());
            }
            fill(prompts.edit, &[("instruction", instruction), ("document", document), ("answer_in", prompts.answer_in)])
        }
    };
    Ok(vec![system, Msg::User(user)])
}

/// Turns one answer into the document the window applies. Structural problems are errors that say
/// what the model got wrong, so a bad answer is a message, not a half-written document.
pub fn interpret(_request: &Request, answer: &str) -> Result<Outcome, String> {
    let answer: DocAnswer = local_ai::extract::extract_json(answer)?;
    if answer.blocks.is_empty() {
        return Err("the model did not answer any block".into());
    }
    let title = answer.title.trim().to_string();
    let mut blocks = Vec::with_capacity(answer.blocks.len());
    for (index, block) in answer.blocks.into_iter().enumerate().take(MAX_BLOCKS) {
        let kind = block.kind.trim().to_ascii_lowercase();
        match kind.as_str() {
            "heading" | "title" => {
                let text = block.text.trim().to_string();
                if text.is_empty() {
                    return Err(format!("the model answered heading {} without text", index + 1));
                }
                blocks.push(Block::Heading { level: block.level.unwrap_or(1).clamp(1, 3), text });
            }
            "list" | "bullets" => {
                let items: Vec<String> = block.items.map(|items| items.into_list()).unwrap_or_default();
                let items: Vec<String> = items.into_iter().map(|item| item.trim().to_string()).filter(|item| !item.is_empty()).collect();
                if items.is_empty() {
                    return Err(format!("the model answered list {} without items", index + 1));
                }
                blocks.push(Block::List { items });
            }
            _ => {
                let text = block.text.trim().to_string();
                if text.is_empty() {
                    return Err(format!("the model answered block {} without text", index + 1));
                }
                blocks.push(Block::Paragraph { text });
            }
        }
    }
    Ok(Outcome::Doc { title, blocks })
}

/// The document's plain text as an answer's blocks would write it: one paragraph per line, list
/// items each on their own line. This is what `EditDoc` sends and what the window pastes, so both
/// sides agree on what a block boundary is.
pub fn body_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            Block::Heading { text, .. } | Block::Paragraph { text } => {
                out.push_str(text);
                out.push('\n');
            }
            Block::List { items } => {
                for item in items {
                    out.push_str(item);
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[derive(Deserialize)]
struct DocAnswer {
    #[serde(default)]
    title: String,
    #[serde(default)]
    blocks: Vec<AnswerBlock>,
}

#[derive(Deserialize)]
struct AnswerBlock {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    level: Option<u8>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    items: Option<Items>,
}

/// A list's items, from either shape a model answers (a list, or one string with lines).
#[derive(Deserialize)]
#[serde(untagged)]
enum Items {
    One(String),
    Many(Vec<String>),
}

impl Items {
    fn into_list(self) -> Vec<String> {
        match self {
            Items::One(text) => text.lines().map(str::to_string).collect(),
            Items::Many(items) => items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate() -> Request {
        Request::GenerateDoc { topic: "protocolo de sepse".into(), language: "português".into(), shape: None }
    }

    #[test]
    fn an_answer_becomes_headings_paragraphs_and_lists() {
        let answer = r#"{"title":"Protocolo","blocks":[{"kind":"heading","level":1,"text":"Sepse"},{"kind":"paragraph","text":"Definição."},{"kind":"list","items":["febre","leucocitose"]}]}"#;
        match interpret(&generate(), answer).expect("a document") {
            Outcome::Doc { title, blocks } => {
                assert_eq!(title, "Protocolo");
                assert_eq!(blocks.len(), 3);
                assert_eq!(blocks[0], Block::Heading { level: 1, text: "Sepse".into() });
                assert_eq!(blocks[1], Block::Paragraph { text: "Definição.".into() });
                assert_eq!(blocks[2], Block::List { items: vec!["febre".into(), "leucocitose".into()] });
            }
        }
    }

    /// The extractor reads through reasoning blocks and fences; the actions do not care.
    #[test]
    fn the_extractor_reads_through_reasoning_and_fences() {
        let answer = "<think>{nope}</think>\n```json\n{\"blocks\":[{\"kind\":\"paragraph\",\"text\":\"x\"}]}\n```";
        assert!(matches!(interpret(&generate(), answer), Ok(Outcome::Doc { .. })));
    }

    #[test]
    fn an_unknown_kind_is_a_paragraph_and_a_list_can_be_one_text() {
        let answer = r#"{"blocks":[{"kind":"quote","text":"citação"},{"kind":"list","items":"um\ndois"}]}"#;
        match interpret(&generate(), answer).expect("a document") {
            Outcome::Doc { blocks, .. } => {
                assert_eq!(blocks[0], Block::Paragraph { text: "citação".into() });
                assert_eq!(blocks[1], Block::List { items: vec!["um".into(), "dois".into()] });
            }
        }
    }

    #[test]
    fn a_heading_level_is_clamped_and_a_missing_block_is_refused() {
        let answer = r#"{"blocks":[{"kind":"heading","level":9,"text":"T"}]}"#;
        match interpret(&generate(), answer).expect("a document") {
            Outcome::Doc { blocks, .. } => assert_eq!(blocks[0], Block::Heading { level: 3, text: "T".into() }),
        }
        assert!(interpret(&generate(), r#"{"blocks":[]}"#).is_err());
        assert!(interpret(&generate(), r#"{"blocks":[{"kind":"paragraph","text":"  "}]}"#).is_err());
    }

    #[test]
    fn an_empty_topic_or_instruction_is_refused_before_the_request() {
        assert!(messages(&Request::GenerateDoc { topic: "  ".into(), language: "en".into(), shape: None }, "en").is_err());
        assert!(messages(&Request::EditDoc { instruction: " ".into(), document: "x".into() }, "en").is_err());
    }

    #[test]
    fn the_messages_carry_the_contract_and_the_language() {
        let request = Request::GenerateDoc { topic: "sepse".into(), language: "português".into(), shape: Some("a report".into()) };
        let messages = messages(&request, "pt-br").expect("messages");
        let Msg::System(system) = &messages[0] else { panic!("system turn") };
        assert!(system.contains("português (Brasil)"), "the answer language travels: {system}");
        let Msg::User(user) = &messages[1] else { panic!("user turn") };
        assert!(user.contains("\"blocks\"") && user.contains("sepse"));
        assert!(user.contains("português"), "the requested document language travels: {user}");
    }

    /// What the window pastes is exactly what `EditDoc` would read back from the document.
    #[test]
    fn the_body_text_lays_blocks_out_one_per_line() {
        let blocks = vec![
            Block::Heading { level: 1, text: "Título".into() },
            Block::Paragraph { text: "Um parágrafo.".into() },
            Block::List { items: vec!["a".into(), "b".into()] },
        ];
        assert_eq!(body_text(&blocks), "Título\nUm parágrafo.\na\nb\n");
    }
}
