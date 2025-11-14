use anyhow::Result;

use crate::ctx;

// ~once again
pub fn get_username() -> Result<String> {
    std::env::var("USER").map_err(ctx!("can't get USER environment variable"))
}
