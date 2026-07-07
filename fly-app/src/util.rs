use crate::error::ErrorResponse;
use actix_web::HttpResponse;

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

    Ok(format!("ORDER BY {}\n", clauses.join(", ")))
}
