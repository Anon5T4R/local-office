//! The prompts, localized, with the JSON contract written out. The contract's keys (`title`,
//! `blocks`, `kind`, `level`, `text`, `items`) are never translated: the extractor and the window
//! read them by name.

/// One language's prompt set.
pub struct Prompts {
    pub system: &'static str,
    pub answer_in: &'static str,
    pub generate: &'static str,
    pub edit: &'static str,
}

/// The prompts for a language code (`pt`, `pt-br`, `es`, `ja`, ...): Portuguese, Spanish and
/// Japanese when we have them, English otherwise (the source language).
pub fn for_lang(lang: &str) -> &'static Prompts {
    match lang.split(['-', '_']).next().unwrap_or("en").to_ascii_lowercase().as_str() {
        "pt" => &PT,
        "es" => &ES,
        "ja" => &JA,
        _ => &EN,
    }
}

/// Fills `{name}` placeholders in one pass. `{{` and `}}` are literal braces, and a substituted
/// value is never expanded again, so a document that happens to contain braces is safe.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for n in chars.by_ref() {
                    if n == '}' {
                        closed = true;
                        break;
                    }
                    name.push(n);
                }
                match args.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) if closed => out.push_str(value),
                    _ => {
                        out.push('{');
                        out.push_str(&name);
                        if closed {
                            out.push('}');
                        }
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

static EN: Prompts = Prompts {
    system: "You are a writing assistant. Answer in {answer_in} unless the task asks for another \
             language. Return only the JSON object described: no prose, no markdown, no code fences.",
    answer_in: "English",
    generate: "Write a document about: {topic}\n\
               Write it in {language}.\n\n\
               Return one JSON object with exactly this shape:\n\
               {{\"title\": \"a short document title\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"a heading\"}}, {{\"kind\": \"paragraph\", \"text\": \"a paragraph\"}}, {{\"kind\": \"list\", \"items\": [\"first item\", \"second item\"]}}]}}\n\n\
               Rules:\n\
               - \"kind\" is one of: heading (with \"level\" 1 to 3), paragraph or list;\n\
               - a block carries \"text\", a list carries \"items\" (one string each);\n\
               - write real content, in whole sentences, in {language}; never placeholders;\n\
               - no prose outside the JSON.",
    edit: "Rewrite this document.\n\n\
           Instruction: {instruction}\n\n\
           Document:\n{document}\n\n\
           Return one JSON object with exactly this shape:\n\
           {{\"title\": \"the document title\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"...\"}}, {{\"kind\": \"paragraph\", \"text\": \"...\"}}]}}\n\n\
           Rules:\n\
           - return the WHOLE document after the change, not a diff and not only the changed part;\n\
           - keep everything the instruction does not ask to change, including the language, the order and the headings;\n\
           - \"kind\" is heading (level 1 to 3), paragraph or list; a list carries \"items\";\n\
           - no prose outside the JSON.",
};

static PT: Prompts = Prompts {
    system: "Você é um assistente de redação. Responda em {answer_in} a menos que a tarefa peça \
             outro idioma. Devolva apenas o objeto JSON descrito: sem prosa, sem markdown, sem \
             cercas de código.",
    answer_in: "português (Brasil)",
    generate: "Escreva um documento sobre: {topic}\n\
               Escreva em {language}.\n\n\
               Devolva um objeto JSON exatamente com esta forma:\n\
               {{\"title\": \"um título curto\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"um título de seção\"}}, {{\"kind\": \"paragraph\", \"text\": \"um parágrafo\"}}, {{\"kind\": \"list\", \"items\": [\"primeiro item\", \"segundo item\"]}}]}}\n\n\
               Regras:\n\
               - \"kind\" é um de: heading (com \"level\" de 1 a 3), paragraph ou list;\n\
               - um bloco traz \"text\"; uma lista traz \"items\" (uma string cada);\n\
               - escreva conteúdo real, em frases inteiras, em {language}; nunca placeholders;\n\
               - sem prosa fora do JSON.",
    edit: "Reescreva este documento.\n\n\
           Instrução: {instruction}\n\n\
           Documento:\n{document}\n\n\
           Devolva um objeto JSON exatamente com esta forma:\n\
           {{\"title\": \"o título do documento\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"...\"}}, {{\"kind\": \"paragraph\", \"text\": \"...\"}}]}}\n\n\
           Regras:\n\
           - devolva o documento INTEIRO depois da mudança, não um diff nem só o trecho mudado;\n\
           - mantenha tudo o que a instrução não pede para mudar, incluindo o idioma, a ordem e os títulos;\n\
           - \"kind\" é heading (nível 1 a 3), paragraph ou list; uma lista traz \"items\";\n\
           - sem prosa fora do JSON.",
};

static ES: Prompts = Prompts {
    system: "Eres un asistente de redacción. Responde en {answer_in} salvo que la tarea pida otro \
             idioma. Devuelve solo el objeto JSON descrito: sin prosa, sin markdown, sin bloques de \
             código.",
    answer_in: "español",
    generate: "Escribe un documento sobre: {topic}\n\
               Escríbelo en {language}.\n\n\
               Devuelve un objeto JSON exactamente con esta forma:\n\
               {{\"title\": \"un título corto\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"un título de sección\"}}, {{\"kind\": \"paragraph\", \"text\": \"un párrafo\"}}, {{\"kind\": \"list\", \"items\": [\"primer elemento\", \"segundo elemento\"]}}]}}\n\n\
               Reglas:\n\
               - \"kind\" es uno de: heading (con \"level\" de 1 a 3), paragraph o list;\n\
               - un bloque lleva \"text\"; una lista lleva \"items\" (una cadena cada uno);\n\
               - escribe contenido real, en frases completas, en {language}; nunca marcadores;\n\
               - sin prosa fuera del JSON.",
    edit: "Reescribe este documento.\n\n\
           Instrucción: {instruction}\n\n\
           Documento:\n{document}\n\n\
           Devuelve un objeto JSON exactamente con esta forma:\n\
           {{\"title\": \"el título del documento\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"...\"}}, {{\"kind\": \"paragraph\", \"text\": \"...\"}}]}}\n\n\
           Reglas:\n\
           - devuelve el documento ENTERO tras el cambio, no un diff ni solo la parte cambiada;\n\
           - mantén todo lo que la instrucción no pide cambiar, incluidos el idioma, el orden y los títulos;\n\
           - \"kind\" es heading (nivel 1 a 3), paragraph o list; una lista lleva \"items\";\n\
           - sin prosa fuera del JSON.",
};

static JA: Prompts = Prompts {
    system: "あなたは文章作成アシスタントです。タスクが別の言語を求めない限り {answer_in} で答えてください。\
             説明した JSON オブジェクトだけを返してください: 文章も markdown もコードフェンスも不要です。",
    answer_in: "日本語",
    generate: "{topic} についての文書を書いてください。\n\
               {language} で書いてください。\n\n\
               次の形の JSON オブジェクトを 1 つだけ返してください:\n\
               {{\"title\": \"短い文書の見出し\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"節の見出し\"}}, {{\"kind\": \"paragraph\", \"text\": \"段落\"}}, {{\"kind\": \"list\", \"items\": [\"1 つ目\", \"2 つ目\"]}}]}}\n\n\
               ルール:\n\
               - \"kind\" は heading (\"level\" は 1〜3)、paragraph、list のいずれか。\n\
               - ブロックは \"text\"、リストは \"items\" (各 1 文字列) を持つ。\n\
               - {language} で、完全な文の実際の内容を書く。プレースホルダーは不可。\n\
               - JSON の外に文章を書かない。",
    edit: "この文書を書き換えてください。\n\n\
           指示: {instruction}\n\n\
           文書:\n{document}\n\n\
           次の形の JSON オブジェクトを 1 つだけ返してください:\n\
           {{\"title\": \"文書の見出し\", \"blocks\": [{{\"kind\": \"heading\", \"level\": 1, \"text\": \"...\"}}, {{\"kind\": \"paragraph\", \"text\": \"...\"}}]}}\n\n\
           ルール:\n\
           - 変更後の文書全体を返す (差分や変更部分だけではない)。\n\
           - 指示が求めていないものは、言語・順序・見出しを含めてそのまま残す。\n\
           - \"kind\" は heading (1〜3)、paragraph、list。リストは \"items\" を持つ。\n\
           - JSON の外に文章を書かない。",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_the_json_contract() {
        for prompts in [&EN, &PT, &ES, &JA] {
            assert!(prompts.generate.contains("\"title\"") && prompts.generate.contains("\"blocks\""));
            assert!(prompts.generate.contains("\"kind\""), "no block vocabulary");
            assert!(prompts.edit.contains("\"document\"") || prompts.edit.contains("{document}"));
            assert!(prompts.edit.contains("\"items\"") || prompts.edit.contains("items"));
        }
    }

    #[test]
    fn the_language_pick_falls_back_to_english() {
        assert_eq!(for_lang("pt-br").answer_in, "português (Brasil)");
        assert_eq!(for_lang("es").answer_in, "español");
        assert_eq!(for_lang("ja").answer_in, "日本語");
        assert_eq!(for_lang("de").answer_in, "English");
        assert_eq!(for_lang("").answer_in, "English");
    }

    #[test]
    fn braces_are_literal_and_values_are_not_expanded_again() {
        assert_eq!(fill("a {{b}} c", &[]), "a {b} c");
        assert_eq!(fill("{topic} {topic}", &[("topic", "{topic}")]), "{topic} {topic}");
        let text = fill(EN.generate, &[("topic", "sepse"), ("language", "Portuguese"), ("answer_in", "English")]);
        assert!(text.contains("{\"title\": \"a short document title\", \"blocks\":"), "{text}");
        assert!(!text.contains("{{"), "{text}");
    }
}
