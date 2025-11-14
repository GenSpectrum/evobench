use crate::serde::allowed_env_var::AllowEnvVar;

pub const EVOBENCH_ENV_VARS: &[&str] = &[
    "EVOBENCH_LOG",
    "BENCH_OUTPUT_LOG",
    "COMMIT_ID",
    "DATASET_DIR",
];

pub fn is_evobench_env_var(s: &str) -> bool {
    EVOBENCH_ENV_VARS.contains(&s)
}

/// A parameter for `AllowedEnvVar` that checks that the variable is
/// not going to conflict with one of the built-in evobench env vars
/// (in the future perhaps also check for things like USER?)
#[derive(Debug)]
pub struct AllowableCustomEnvVar;
impl AllowEnvVar for AllowableCustomEnvVar {
    const MAX_ENV_VAR_NAME_LEN: usize = 80;

    fn allow_env_var(s: &str) -> bool {
        !is_evobench_env_var(s)
    }

    fn expecting() -> String {
        format!(
            "a variable name that is *not* any of {}",
            EVOBENCH_ENV_VARS.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::serde::allowed_env_var::AllowedEnvVar;

    use super::*;

    #[test]
    fn t_allowable_custom_env_var_name() {
        let allow = AllowableCustomEnvVar::allow_env_var;
        assert!(allow("FOO"));
        // We don't care whether the user decides to use unorthodox
        // variable names
        assert!(allow("foo"));
        assert!(allow("%&/',é\nhmm"));
        assert!(allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));
        // Too long, but have to rely on `AllowedEnvVar` to get the
        // `MAX_ENV_VAR_NAME_LEN` constant checked
        assert!(allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));

        let allow =
            |s: &str| -> bool { AllowedEnvVar::<AllowableCustomEnvVar>::from_str(s).is_ok() };

        assert!(allow("foo"));
        assert!(allow("%&/',é\nhmm"));
        assert!(allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));

        // Problems caughtby `AllowedEnvVar::from_str`
        assert!(!allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));
        assert!(!allow("A\0B"));
        assert!(!allow("foo=bar"));
        assert!(!allow("EVOBENCH_LOG"));

        assert_eq!(
            AllowedEnvVar::<AllowableCustomEnvVar>::from_str("EVOBENCH_LOG")
                .err()
                .unwrap()
                .to_string(),
            "AllowableCustomEnvVar env variable \"EVOBENCH_LOG\" is reserved, expecting a variable name \
             that is *not* any of EVOBENCH_LOG, BENCH_OUTPUT_LOG, COMMIT_ID, DATASET_DIR"
        );
    }
}

// Can't make this const easily, but doesn't matter. It'll catch bugs
// on the first job run.
pub fn assert_evobench_env_var(s: &str) -> &str {
    if is_evobench_env_var(s) {
        s
    } else {
        panic!("Not a known EVOBENCH_ENV_VARS entry: {s:?}")
    }
}
