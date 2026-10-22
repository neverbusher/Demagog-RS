use crate::domain::ChunkConfig;
use crate::ports::TextChunker;

pub struct SentenceAwareChunker;

impl TextChunker for SentenceAwareChunker {
    fn split_into_chunks(&self, text: &str, config: &ChunkConfig) -> Vec<String> {
        let max_chars = config.max_chars.max(80);
        let mut chunks = Vec::new();
        let mut current = String::new();

        for raw_para in text.lines() {
            let para = raw_para.trim();
            if para.is_empty() {
                continue;
            }

            if config.preserve_paragraphs && !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }

            for sentence in split_into_sentences(para) {
                let sent_chars = sentence.chars().count();
                if sent_chars > max_chars {
                    if !current.is_empty() {
                        chunks.push(std::mem::take(&mut current));
                    }
                    for sub in split_long_sentence(&sentence, max_chars) {
                        chunks.push(sub);
                    }
                    continue;
                }

                let cur_chars = current.chars().count();
                let sep_len = if current.is_empty() { 0 } else { 1 };
                if cur_chars + sep_len + sent_chars <= max_chars {
                    if !current.is_empty() {
                        if config.preserve_paragraphs {
                            current.push(' ');
                        } else {
                            current.push('\n');
                        }
                    }
                    current.push_str(&sentence);
                } else {
                    if !current.is_empty() {
                        chunks.push(std::mem::take(&mut current));
                    }
                    current.push_str(&sentence);
                }
            }
        }

        if !current.trim().is_empty() {
            chunks.push(current);
        }

        chunks
    }
}

fn split_into_sentences(paragraph: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = paragraph.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        cur.push(c);
        if matches!(c, '.' | '!' | '?' | '…') {
            while i + 1 < chars.len()
                && matches!(chars[i + 1], '.' | '!' | '?' | '…' | '"' | '»' | ')')
            {
                i += 1;
                cur.push(chars[i]);
            }
            if i + 1 == chars.len() || chars[i + 1].is_whitespace() {
                let trimmed = cur.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                cur.clear();
            }
        }
        i += 1;
    }

    let rem = cur.trim().to_string();
    if !rem.is_empty() {
        sentences.push(rem);
    }
    sentences
}

fn split_long_sentence(sentence: &str, max_chars: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();

    for word in sentence.split_whitespace() {
        let w_len = word.chars().count();
        let c_len = cur.chars().count();
        if !cur.is_empty() && c_len + 1 + w_len > max_chars {
            parts.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }

    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}
