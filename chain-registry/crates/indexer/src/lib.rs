pub mod api;
pub mod apply;
pub mod bounty;
pub mod source;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod test_db;
