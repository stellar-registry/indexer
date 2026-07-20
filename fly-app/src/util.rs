use crate::error::ErrorResponse;
use actix_web::HttpResponse;

const DEFAULT_GITHUB_REF: &str = "main";

pub fn build_sort_spec(
    sort_by: Vec<String>,
    descending: Vec<bool>,
    allowed_columns: &[&str],
) -> Result<String, HttpResponse> {
    if sort_by.len() != descending.len() {
        return Err(HttpResponse::BadRequest().json(ErrorResponse {
            error: format!(
                "sort_by has {} entries but descending has {} entries; they must match",
                sort_by.len(),
                descending.len()
            ),
        }));
    }

    if sort_by.is_empty() {
        return Ok(String::new());
    }

    let mut clauses = Vec::with_capacity(sort_by.len());

    for (column, desc) in sort_by.iter().zip(descending.iter()) {
        if !allowed_columns.contains(&column.as_str()) {
            continue;
        }

        let direction = if *desc { "DESC" } else { "ASC" };
        clauses.push(format!("{} {}", column, direction));
    }

    if clauses.is_empty() {
        return Ok(String::new());
    }

    Ok(clauses.join(", "))
}

pub fn parse_source_repo(source_repo: &str) -> String {
    let source_repo = source_repo.trim();
    if source_repo.is_empty() {
        return String::new();
    }

    if let Some(repo_path) = source_repo.strip_prefix("github:") {
        if let Some(normalized) = normalize_github_path(repo_path) {
            return normalized;
        }
    }

    if let Some(repo_path) = source_repo
        .strip_prefix("https://github.com/")
        .or_else(|| source_repo.strip_prefix("http://github.com/"))
        .or_else(|| source_repo.strip_prefix("github.com/"))
    {
        if let Some(normalized) = normalize_github_path(repo_path) {
            return normalized;
        }
    }

    source_repo.to_string()
}

fn normalize_github_path(repo_path: &str) -> Option<String> {
    let repo_path = strip_query_and_fragment(repo_path);
    let segments: Vec<&str> = repo_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();

    if segments.len() < 2 {
        return None;
    }

    let owner = segments[0];
    let repo = segments[1].trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    if segments.len() >= 4 && segments[2] == "tree" && !segments[3].is_empty() {
        return Some(build_tree_url(owner, repo, segments[3], &segments[4..]));
    }

    // `blob` URLs are still resolvable to a path under a branch/tag ref.
    if segments.len() >= 4 && segments[2] == "blob" && !segments[3].is_empty() {
        return Some(build_tree_url(owner, repo, segments[3], &segments[4..]));
    }

    Some(build_tree_url(
        owner,
        repo,
        DEFAULT_GITHUB_REF,
        &segments[2..],
    ))
}

fn build_tree_url(owner: &str, repo: &str, reference: &str, rest: &[&str]) -> String {
    let mut normalized = format!("https://github.com/{owner}/{repo}/tree/{reference}");
    if !rest.is_empty() {
        normalized.push('/');
        normalized.push_str(&rest.join("/"));
    }
    normalized
}

fn strip_query_and_fragment(value: &str) -> &str {
    let mut end = value.len();

    if let Some(index) = value.find('?') {
        end = end.min(index);
    }

    if let Some(index) = value.find('#') {
        end = end.min(index);
    }

    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::parse_source_repo;

    #[test]
    fn parses_github_shorthand_repo() {
        let normalized = parse_source_repo("github:stellar-registry/contracts");
        assert_eq!(
            normalized,
            "https://github.com/stellar-registry/contracts/tree/main"
        );
    }

    #[test]
    fn keeps_github_tree_url_shape() {
        let normalized = parse_source_repo(
            "https://github.com/theahaco/stellar-contracts-OZ/tree/main/combinations/ft-allowlist-capped-pausable",
        );
        assert_eq!(
            normalized,
            "https://github.com/theahaco/stellar-contracts-OZ/tree/main/combinations/ft-allowlist-capped-pausable"
        );
    }

    #[test]
    fn converts_plain_github_repo_url() {
        let normalized = parse_source_repo("https://github.com/stellar/rs-soroban-sdk");
        assert_eq!(
            normalized,
            "https://github.com/stellar/rs-soroban-sdk/tree/main"
        );
    }

    #[test]
    fn leaves_non_github_source_repo_unchanged() {
        let input = "https://gitlab.com/acme/contracts";
        assert_eq!(parse_source_repo(input), input.to_string());
    }
}
