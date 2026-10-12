//! Fixed checks on the words a model writes for a customer, applied before anything is shown:
//! one plain sentence (R23), no money (R02: the AI never states a price, a premium or a sum
//! insured), no protected attribute (R19). The words and limits come from the language packs
//! (`config/languages/*.yaml`, `language::Languages`); nothing here is specific to a language.

/// The shape of one plain sentence in a language.
#[derive(Debug, Clone, Default)]
pub struct SentenceRules {
    pub terminators: Vec<char>,
    /// For languages written with spaces.
    pub max_words: Option<usize>,
    /// For languages written without spaces.
    pub max_chars: Option<usize>,
}

/// R23: one plain sentence. Non-empty, one line, within the language's length, ending with one of
/// its terminators and with no other sentence boundary inside: a terminator followed by a space,
/// or a full-width one followed by anything ("8.5" and "e.g." without a space are not
/// boundaries).
pub fn is_one_sentence(text: &str, rules: &SentenceRules) -> bool {
    let t = text.trim();
    let Some(last) = t.chars().last() else {
        return false;
    };
    if !rules.terminators.contains(&last) || t.contains('\n') {
        return false;
    }
    if rules
        .max_words
        .is_some_and(|m| t.split_whitespace().count() > m)
        || rules.max_chars.is_some_and(|m| t.chars().count() > m)
    {
        return false;
    }
    let body: Vec<char> = t.chars().collect();
    let inner = &body[..body.len() - 1];
    let boundary = inner.iter().enumerate().any(|(i, c)| {
        rules.terminators.contains(c)
            && (!c.is_ascii() || inner.get(i + 1).is_some_and(|n| n.is_whitespace()))
    });
    !boundary && !t.ends_with("..")
}

/// Terms a check looks for. A term of letters, digits, hyphens and spaces matches whole words;
/// any other term (a sign, or a word of a language written without spaces) matches anywhere.
#[derive(Debug, Clone, Default)]
pub struct Terms {
    words: Vec<Vec<String>>,
    anywhere: Vec<String>,
}

impl Terms {
    pub fn new<'a>(terms: impl IntoIterator<Item = &'a str>) -> Terms {
        let mut out = Terms::default();
        for t in terms {
            let t = t.trim().to_lowercase();
            if t.is_empty() {
                continue;
            }
            if t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ' ')
            {
                out.words
                    .push(t.split_whitespace().map(str::to_string).collect());
            } else {
                out.anywhere.push(t);
            }
        }
        out
    }

    pub fn found_in(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        if self.anywhere.iter().any(|t| lower.contains(t.as_str())) {
            return true;
        }
        let words = words(&lower);
        self.words.iter().any(|phrase| {
            words
                .windows(phrase.len())
                .any(|w| w.iter().zip(phrase).all(|(a, b)| *a == b.as_str()))
        })
    }
}

fn words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// The whole numbers in a text, reading `1,000` as one number.
pub fn numbers(text: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let joins =
            *c == ',' && !current.is_empty() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() {
            current.push(*c);
        } else if !joins {
            if let Ok(n) = current.parse() {
                out.push(n);
            }
            current.clear();
        }
    }
    if let Ok(n) = current.parse() {
        out.push(n);
    }
    out
}

/// R02: whether the text states money: a currency term, or one of `amounts` (major units).
pub fn states_money(text: &str, currency: &Terms, amounts: &[i64]) -> bool {
    currency.found_in(text)
        || numbers(text)
            .into_iter()
            .any(|n| amounts.iter().any(|a| *a == n && *a != 0))
}

/// A snake_case id, such as a question id or a task type.
pub fn is_snake_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 48
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}
