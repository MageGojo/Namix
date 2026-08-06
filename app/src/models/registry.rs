//! 全部 Toasty 模型注册表（Boot / seed / toasty-cli）。
//!
//! 新增 Model 后记得加进 `models!`，否则不会建表。
//!
//! ## 相对 Laravel Eloquent
//!
//! 已对齐：表名、主键、关联（1:1 / 1:N / N:N）、timestamps、`email_verified_at`、
//! pivot 唯一约束、密码字段不序列化。
//!
//! Toasty / 本案例**不做**（或放到别层）：
//! - SoftDeletes → 无一等支持时用 `deleted_at` 字段 + 查询过滤
//! - `$fillable` / `$casts` / Accessor → Service 或手写 `impl`
//! - Factory / Seeder → `app/src/seeders`

use super::login_log::LoginLog;
use super::note::Note;
use super::post::Post;
use super::post_tag::PostTag;
use super::profile::Profile;
use super::tag::Tag;
use super::user::User;

pub fn model_set() -> toasty::ModelSet {
    toasty::models!(User, Profile, Post, Tag, PostTag, LoginLog, Note)
}
