//! Masks credential-looking words in text that comes from other programs
//! (ai-usagebar errors) before it is logged, saved to the status file, or
//! shown in the sidebar.

const MASK: &str = "[redacted]";
const SECRET_KEY_SUFFIXES: [&str; 5] = ["key", "token", "secret", "password", "authorization"];
const SECRET_PREFIXES: [&str; 6] = ["sk-", "ghp_", "gho_", "ghu_", "github_pat_", "xox"];
const MIN_OPAQUE_LEN: usize = 32;

pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_bearer = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        let spacing = &piece[word.len()..];
        if word.is_empty() {
            out.push_str(spacing);
            continue;
        }
        if after_bearer {
            out.push_str(MASK);
        } else {
            out.push_str(&redact_word(word));
        }
        out.push_str(spacing);
        after_bearer = word.eq_ignore_ascii_case("bearer");
    }
    out
}

fn redact_word(word: &str) -> String {
    if let Some((key, value)) = word.split_once(['=', ':'])
        && !value.is_empty()
        && is_secret_key(key)
    {
        let separator = &word[key.len()..key.len() + 1];
        return format!("{key}{separator}{MASK}");
    }
    let core = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && !"-_".contains(c));
    if core.is_empty() {
        return word.to_string();
    }
    let lower = core.to_ascii_lowercase();
    if SECRET_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || looks_opaque(core)
    {
        return word.replacen(core, MASK, 1);
    }
    word.to_string()
}

fn is_secret_key(key: &str) -> bool {
    let key = key
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    SECRET_KEY_SUFFIXES
        .iter()
        .any(|suffix| key.ends_with(suffix))
}

/// Long unbroken runs of letters and digits, as in API keys and JWTs.
fn looks_opaque(core: &str) -> bool {
    core.len() >= MIN_OPAQUE_LEN
        && core
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+/=".contains(c))
        && !core.contains('/')
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_errors_pass_through() {
        let text = "credentials error: Zai: no API key. Either set an API key in a valid \
                    environment variable or set `api_key` under [zai] in ~/.config/x.toml.";
        assert_eq!(redact(text), text);
        assert_eq!(
            redact("HTTP 429: rate limited; next attempt in 4m"),
            "HTTP 429: rate limited; next attempt in 4m"
        );
    }

    #[test]
    fn bearer_tokens_are_masked() {
        assert_eq!(
            redact("401 for Authorization: Bearer abc.def.ghi, retry"),
            "401 for Authorization: Bearer [redacted] retry"
        );
    }

    #[test]
    fn key_value_pairs_with_secret_names_are_masked() {
        assert_eq!(
            redact("bad api_key=sk12345 given"),
            "bad api_key=[redacted] given"
        );
        assert_eq!(redact("x-api-key:abcdef"), "x-api-key:[redacted]");
        assert_eq!(redact("refresh_token=xyz;"), "refresh_token=[redacted]");
        assert_eq!(redact("status=401"), "status=401");
    }

    #[test]
    fn known_token_prefixes_are_masked_inside_punctuation() {
        assert_eq!(
            redact("key \"sk-ant-abc123\" rejected"),
            "key \"[redacted]\" rejected"
        );
        assert_eq!(redact("(ghp_abcdef)"), "([redacted])");
    }

    #[test]
    fn long_opaque_strings_are_masked_but_paths_and_words_are_not() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(
            redact(&format!("token {jwt} expired")),
            "token [redacted] expired"
        );
        assert_eq!(
            redact("/Users/someone/Library/Application Support/ai-usagebar/config.toml"),
            "/Users/someone/Library/Application Support/ai-usagebar/config.toml"
        );
        assert_eq!(redact("internationalization"), "internationalization");
    }

    #[test]
    fn whitespace_is_preserved() {
        assert_eq!(redact("a  b\nc\t"), "a  b\nc\t");
    }
}
