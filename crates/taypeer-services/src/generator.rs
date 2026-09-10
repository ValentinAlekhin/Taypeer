//! Offline password and passphrase generation, separate from manual-password scoring.

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use zeroize::Zeroizing;

const SIMILAR: &str = "Il1O0o";
const WORDLIST: &str = include_str!("../../../resources/eff-long-wordlist.txt");

/// Exact character-selection options. Enabling a set does not force its presence.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PasswordOptions {
    /// Number of independently sampled ASCII characters, 1 through 256.
    pub length: u16,
    /// Include A-Z.
    pub uppercase: bool,
    /// Include a-z.
    pub lowercase: bool,
    /// Include 0-9.
    pub digits: bool,
    /// Include printable ASCII punctuation, excluding spaces.
    pub punctuation: bool,
    /// Exclude I, l, 1, O, 0 and o.
    pub exclude_similar: bool,
    /// Additional exact characters to remove from the alphabet.
    pub exclude: String,
}
impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            length: 30,
            uppercase: true,
            lowercase: true,
            digits: true,
            punctuation: true,
            exclude_similar: false,
            exclude: String::new(),
        }
    }
}

/// A generated secret with entropy calculated from the known sampling method.
pub struct GeneratedSecret {
    value: Zeroizing<String>,
    entropy_bits: f64,
}
impl std::fmt::Debug for GeneratedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GeneratedSecret([REDACTED])")
    }
}
impl GeneratedSecret {
    /// Explicitly borrow the result for insertion or reveal.
    pub fn expose(&self) -> &str {
        &self.value
    }
    /// Entropy from uniform independent choices; not a score for manually entered text.
    pub fn entropy_bits(&self) -> f64 {
        self.entropy_bits
    }
}

/// Non-secret generation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorError {
    /// Character or word count is outside the product range.
    Length,
    /// All selected characters were excluded.
    EmptyAlphabet,
    /// The operating system could not supply cryptographic randomness.
    Random,
    /// An output allocation could not be represented or reserved.
    Capacity,
}
impl std::fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for GeneratorError {}

/// Generate using the operating system cryptographic random source.
pub fn password(options: &PasswordOptions) -> Result<GeneratedSecret, GeneratorError> {
    password_with(options, &mut OsRng)
}

fn password_with(
    options: &PasswordOptions,
    random: &mut impl RngCore,
) -> Result<GeneratedSecret, GeneratorError> {
    if !(1..=256).contains(&options.length) {
        return Err(GeneratorError::Length);
    }
    let alphabet: Vec<_> = (b'!'..=b'~')
        .filter(|&byte| {
            let enabled = match byte {
                b'A'..=b'Z' => options.uppercase,
                b'a'..=b'z' => options.lowercase,
                b'0'..=b'9' => options.digits,
                _ => options.punctuation,
            };
            enabled
                && !options.exclude.contains(char::from(byte))
                && (!options.exclude_similar || !SIMILAR.contains(char::from(byte)))
        })
        .collect();
    if alphabet.is_empty() {
        return Err(GeneratorError::EmptyAlphabet);
    }
    let mut value = Zeroizing::new(String::with_capacity(options.length.into()));
    for _ in 0..options.length {
        value.push(char::from(alphabet[sample(random, alphabet.len() as u32)?]));
    }
    Ok(GeneratedSecret {
        value,
        entropy_bits: f64::from(options.length) * (alphabet.len() as f64).log2(),
    })
}

/// Generate 3 through 20 words from the bundled EFF Long Wordlist, without network access.
/// Words are chosen independently; repetition is allowed and the separator adds no entropy.
pub fn passphrase(word_count: u8, separator: &str) -> Result<GeneratedSecret, GeneratorError> {
    passphrase_with(word_count, separator, &mut OsRng)
}

fn passphrase_with(
    word_count: u8,
    separator: &str,
    random: &mut impl RngCore,
) -> Result<GeneratedSecret, GeneratorError> {
    if !(3..=20).contains(&word_count) {
        return Err(GeneratorError::Length);
    }
    static WORDS: OnceLock<Vec<&str>> = OnceLock::new();
    let words = WORDS.get_or_init(|| {
        WORDLIST
            .lines()
            .map(|line| {
                line.split_once('\t')
                    .expect("the bundled EFF list is validated by tests")
                    .1
            })
            .collect()
    });
    let capacity = separator
        .len()
        .checked_mul(usize::from(word_count - 1))
        .and_then(|n| n.checked_add(usize::from(word_count) * 32))
        .ok_or(GeneratorError::Capacity)?;
    let mut value = Zeroizing::new(String::new());
    value
        .try_reserve(capacity)
        .map_err(|_| GeneratorError::Capacity)?;
    for index in 0..word_count {
        if index != 0 {
            value.push_str(separator);
        }
        value.push_str(words[sample(random, words.len() as u32)?]);
    }
    Ok(GeneratedSecret {
        value,
        entropy_bits: f64::from(word_count) * (words.len() as f64).log2(),
    })
}

fn sample(random: &mut impl RngCore, bound: u32) -> Result<usize, GeneratorError> {
    let threshold = bound.wrapping_neg() % bound;
    loop {
        let mut bytes = [0; 4];
        random
            .try_fill_bytes(&mut bytes)
            .map_err(|_| GeneratorError::Random)?;
        let value = u32::from_le_bytes(bytes);
        if value >= threshold {
            return Ok((value % bound) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::impls;

    struct Fixed(u32);
    impl RngCore for Fixed {
        fn next_u32(&mut self) -> u32 {
            self.0
        }
        fn next_u64(&mut self) -> u64 {
            u64::from(self.0)
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            impls::fill_bytes_via_next(self, dest);
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    #[test]
    fn exact_alphabet_and_known_entropy() {
        let result = password_with(&PasswordOptions::default(), &mut Fixed(94)).unwrap();
        assert_eq!(result.expose(), "!".repeat(30));
        assert!((result.entropy_bits() - 196.63766555032913).abs() < 0.0001);
        let options = PasswordOptions {
            uppercase: false,
            lowercase: false,
            punctuation: false,
            exclude: "012345678".into(),
            length: 256,
            ..Default::default()
        };
        assert_eq!(
            password_with(&options, &mut Fixed(0)).unwrap().expose(),
            "9".repeat(256)
        );
        let options = PasswordOptions {
            exclude: "0123456789".into(),
            ..options
        };
        assert_eq!(
            password_with(&options, &mut Fixed(0)).unwrap_err(),
            GeneratorError::EmptyAlphabet
        );
    }

    #[test]
    fn complete_dictionary_repetition_and_entropy() {
        let words: Vec<_> = WORDLIST
            .lines()
            .map(|line| line.split_once('\t').unwrap().1)
            .collect();
        assert_eq!(words.len(), 7776);
        assert_eq!(
            words
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            7776
        );
        let phrase = passphrase_with(6, "-", &mut Fixed(7776 * 100)).unwrap();
        assert_eq!(phrase.expose(), "abacus-abacus-abacus-abacus-abacus-abacus");
        assert!((phrase.entropy_bits() - 77.54887502163469).abs() < 0.0001);
        assert!(passphrase_with(2, "-", &mut Fixed(0)).is_err());
    }
}
