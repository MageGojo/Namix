//! Pagination, filtering, and whitelisted sorting for controller and Action code.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{AppError, Request};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub total: usize,
    pub per_page: usize,
    pub current_page: usize,
    pub last_page: usize,
    pub from: usize,
    pub to: usize,
}

impl<T> Paginator<T> {
    pub fn from_items<I>(items: I, query: &QueryOptions) -> Self
    where
        I: IntoIterator<Item = T>,
    {
        let mut items: Vec<T> = items.into_iter().collect();
        let total = items.len();
        // `QueryOptions::from_request` already validates this range, but
        // callers may construct `QueryOptions` directly. Normalize here as
        // the final invariant boundary so `div_ceil(0)` never panics.
        let per_page = query.per_page.clamp(1, QueryOptions::MAX_PER_PAGE);
        let last_page = total.div_ceil(per_page).max(1);
        let current_page = query.page.min(last_page).max(1);
        let start = (current_page - 1) * per_page;
        let end = (start + per_page).min(total);
        let data = if start < total {
            items.drain(start..end).collect()
        } else {
            Vec::new()
        };
        let from = if data.is_empty() { 0 } else { start + 1 };
        let to = start + data.len();
        Self {
            data,
            total,
            per_page,
            current_page,
            last_page,
            from,
            to,
        }
    }

    pub fn map<U>(self, map: impl FnMut(T) -> U) -> Paginator<U> {
        Paginator {
            data: self.data.into_iter().map(map).collect(),
            total: self.total,
            per_page: self.per_page,
            current_page: self.current_page,
            last_page: self.last_page,
            from: self.from,
            to: self.to,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortField {
    pub field: String,
    pub descending: bool,
}

/// Explicitly permit only database columns that may be exposed for ordering.
#[derive(Clone, Debug, Default)]
pub struct SortWhitelist(BTreeSet<String>);

impl SortWhitelist {
    pub fn new(fields: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self(
            fields
                .into_iter()
                .map(|field| field.as_ref().to_string())
                .collect(),
        )
    }

    pub fn parse(&self, raw: &str) -> Result<SortField, AppError> {
        let (descending, field) = raw
            .strip_prefix('-')
            .map_or((false, raw), |field| (true, field));
        if field.is_empty() || !self.0.contains(field) {
            return Err(AppError::bad_request("sort field is not allowed"));
        }
        Ok(SortField {
            field: field.into(),
            descending,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryOptions {
    pub page: usize,
    pub per_page: usize,
    pub sort: Option<SortField>,
    pub filters: BTreeMap<String, String>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 20,
            sort: None,
            filters: BTreeMap::new(),
        }
    }
}

impl QueryOptions {
    pub const MAX_PER_PAGE: usize = 100;

    pub fn from_request(
        req: &Request,
        sortable: &SortWhitelist,
        filters: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, AppError> {
        let mut options = Self::default();
        if let Some(page) = req.query("page") {
            options.page = parse_positive(&page, "page", 1, usize::MAX)?;
        }
        if let Some(per_page) = req.query("per_page") {
            options.per_page = parse_positive(&per_page, "per_page", 1, Self::MAX_PER_PAGE)?;
        }
        if let Some(sort) = req.query("sort") {
            options.sort = Some(sortable.parse(&sort)?);
        }
        for name in filters {
            let name = name.as_ref();
            let query_name = format!("filter[{name}]");
            if let Some(value) = req.query(&query_name) {
                options.filters.insert(name.into(), value);
            }
        }
        Ok(options)
    }
}

fn parse_positive(raw: &str, name: &str, min: usize, max: usize) -> Result<usize, AppError> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| AppError::validation(name, format!("{name}.integer")))?;
    if !(min..=max).contains(&value) {
        return Err(AppError::validation(name, format!("{name}.between")));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paginates_metadata() {
        let query = QueryOptions {
            page: 2,
            per_page: 2,
            ..QueryOptions::default()
        };
        let page = Paginator::from_items(1..=5, &query);
        assert_eq!(page.data, vec![3, 4]);
        assert_eq!(
            (page.total, page.last_page, page.from, page.to),
            (5, 3, 3, 4)
        );
    }

    #[test]
    fn normalizes_directly_constructed_page_size() {
        let zero = QueryOptions {
            per_page: 0,
            ..QueryOptions::default()
        };
        let page = Paginator::from_items(1..=3, &zero);
        assert_eq!(page.per_page, 1);
        assert_eq!(page.data, vec![1]);

        let oversized = QueryOptions {
            per_page: QueryOptions::MAX_PER_PAGE + 1,
            ..QueryOptions::default()
        };
        let page = Paginator::from_items(0..150, &oversized);
        assert_eq!(page.per_page, QueryOptions::MAX_PER_PAGE);
        assert_eq!(page.data.len(), QueryOptions::MAX_PER_PAGE);
    }
}
