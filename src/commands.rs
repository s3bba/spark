use std::{
    collections::HashSet,
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub(crate) const MAX_VISIBLE_RESULTS: usize = 8;

pub(crate) struct CommandEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

pub(crate) struct SearchResult {
    pub(crate) index: usize,
    score: i32,
    pub(crate) matched_positions: Vec<usize>,
}

struct FuzzyMatch {
    score: i32,
    matched_positions: Vec<usize>,
}

pub(crate) fn load_commands() -> Vec<CommandEntry> {
    let Some(path_value) = env::var_os("PATH") else {
        return Vec::new();
    };

    let mut commands = Vec::new();
    let mut seen = HashSet::new();

    for directory in env::split_paths(&path_value) {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }

            let Some(name) = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(String::from)
            else {
                continue;
            };

            if !seen.insert(name.clone()) {
                continue;
            }

            commands.push(CommandEntry { name, path });
        }
    }

    commands.sort_by(|left, right| left.name.cmp(&right.name));
    commands
}

pub(crate) fn search_results(commands: &[CommandEntry], query: &str) -> Vec<SearchResult> {
    let mut results = commands
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            fuzzy_match(query_filter(query), &entry.name).map(|matched| SearchResult {
                index,
                score: matched.score,
                matched_positions: matched.matched_positions,
            })
        })
        .collect::<Vec<_>>();

    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| commands[left.index].name.cmp(&commands[right.index].name))
    });
    results.truncate(MAX_VISIBLE_RESULTS);
    results
}

pub(crate) fn launch_command(command: &str) -> io::Result<()> {
    println!("Launching: {command}");
    Command::new("sh")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

pub(crate) fn launch_path(path: &Path) -> io::Result<()> {
    println!("Launching: {}", path.display());
    Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

fn query_filter(query: &str) -> &str {
    query.split_whitespace().next().unwrap_or("")
}

fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            matched_positions: Vec::new(),
        });
    }

    let query_chars = query
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    let candidate_lower = candidate_chars
        .iter()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();

    let mut matched_positions = Vec::with_capacity(query_chars.len());
    let mut search_start = 0usize;
    let mut score = 0i32;
    let mut streak = 0i32;

    for query_character in query_chars {
        let Some(relative_index) = candidate_lower[search_start..]
            .iter()
            .position(|candidate_character| *candidate_character == query_character)
        else {
            return None;
        };

        let index = search_start + relative_index;
        score += 10;

        if let Some(previous) = matched_positions.last().copied() {
            if index == previous + 1 {
                streak += 1;
                score += 12 + streak * 3;
            } else {
                streak = 0;
            }
        }

        if index == 0 {
            score += 18;
        } else if matches!(candidate_chars[index - 1], '-' | '_' | '.' | '/' | ' ') {
            score += 10;
        }

        score -= index as i32 / 3;
        matched_positions.push(index);
        search_start = index + 1;
    }

    score -= (candidate_chars
        .len()
        .saturating_sub(matched_positions.len())) as i32;
    Some(FuzzyMatch {
        score,
        matched_positions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str) -> CommandEntry {
        CommandEntry {
            name: name.to_string(),
            path: PathBuf::from(name),
        }
    }

    #[test]
    fn search_only_uses_the_first_word_of_the_query() {
        let commands = vec![command("firefox"), command("fd")];
        let results = search_results(&commands, "firefox https://example.com");

        assert_eq!(results.first().map(|result| result.index), Some(0));
    }

    #[test]
    fn search_prefers_stronger_prefix_matches() {
        let commands = vec![command("ls"), command("false")];
        let results = search_results(&commands, "ls");

        assert_eq!(results.first().map(|result| result.index), Some(0));
    }
}
