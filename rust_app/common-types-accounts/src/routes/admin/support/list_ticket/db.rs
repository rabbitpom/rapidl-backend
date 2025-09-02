use serde::Deserialize;

#[derive(Deserialize)]
pub struct Pagination {
    pub page: usize,
    pub get_total_pages: bool,
    pub get_claimed_only: bool,
}
