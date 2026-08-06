//! 跑全部种子：`cargo run -p app --bin seed`

use namix::db::DbResult;

use super::relations::RelationsSeeder;
use super::users::UsersSeeder;

pub async fn run() -> DbResult<()> {
    UsersSeeder::run().await?;
    RelationsSeeder::run().await?;
    Ok(())
}
