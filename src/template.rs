use crate::error::Error;
use std::collections::BTreeMap;

pub fn render(
    template: &str,
    task_name: &str,
    task_outputs: &BTreeMap<String, String>,
    vars: &BTreeMap<String, String>,
    depends: &[String],
    capture_flags: &BTreeMap<String, bool>,
) -> Result<String, Error> {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            chars.next(); // consume second '{'

            let mut var_name = String::new();
            loop {
                match chars.next() {
                    Some('}') if chars.peek() == Some(&'}') => {
                        chars.next(); // consume second '}'
                        break;
                    }
                    Some(c) => var_name.push(c),
                    None => {
                        return Err(Error::UnresolvedVariable {
                            task: task_name.into(),
                            variable: var_name,
                            hint: "unclosed {{ — missing }}".into(),
                        });
                    }
                }
            }

            let var_name = var_name.trim();
            let value = resolve_variable(
                var_name,
                task_name,
                task_outputs,
                vars,
                depends,
                capture_flags,
            )?;
            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

fn resolve_variable(
    var: &str,
    task_name: &str,
    task_outputs: &BTreeMap<String, String>,
    vars: &BTreeMap<String, String>,
    depends: &[String],
    capture_flags: &BTreeMap<String, bool>,
) -> Result<String, Error> {
    if let Some(rest) = var.strip_prefix("tasks.") {
        let Some(dep_name) = rest.strip_suffix(".stdout") else {
            return Err(Error::UnresolvedVariable {
                task: task_name.into(),
                variable: var.into(),
                hint: "task references must end with .stdout".into(),
            });
        };

        if !depends.contains(&dep_name.to_string()) {
            return Err(Error::InvalidTaskReference {
                task: task_name.into(),
                dependency: dep_name.into(),
                reason: format!("{dep_name:?} is not in this task's depends list"),
            });
        }

        if !capture_flags.get(dep_name).copied().unwrap_or(false) {
            return Err(Error::InvalidTaskReference {
                task: task_name.into(),
                dependency: dep_name.into(),
                reason: format!("{dep_name:?} does not have capture = true"),
            });
        }

        task_outputs
            .get(dep_name)
            .cloned()
            .ok_or_else(|| Error::InvalidTaskReference {
                task: task_name.into(),
                dependency: dep_name.into(),
                reason: format!("{dep_name:?} has not produced output yet"),
            })
    } else if let Some(name) = var.strip_prefix("vars.") {
        vars.get(name)
            .cloned()
            .ok_or_else(|| Error::UnresolvedVariable {
                task: task_name.into(),
                variable: var.into(),
                hint: format!("pass it with --var {name}=\"...\""),
            })
    } else if let Some(name) = var.strip_prefix("env.") {
        std::env::var(name).map_err(|_| Error::UnresolvedVariable {
            task: task_name.into(),
            variable: var.into(),
            hint: format!("environment variable {name} is not set"),
        })
    } else {
        Err(Error::UnresolvedVariable {
            task: task_name.into(),
            variable: var.into(),
            hint: "variables must start with tasks., vars., or env.".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_outputs() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn empty_vars() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn empty_captures() -> BTreeMap<String, bool> {
        BTreeMap::new()
    }

    #[test]
    fn no_placeholders() {
        let result = render(
            "Hello world",
            "test",
            &empty_outputs(),
            &empty_vars(),
            &[],
            &empty_captures(),
        )
        .unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn vars_interpolation() {
        let mut vars = BTreeMap::new();
        vars.insert("name".into(), "Alice".into());
        let result = render(
            "Hello {{vars.name}}",
            "test",
            &empty_outputs(),
            &vars,
            &[],
            &empty_captures(),
        )
        .unwrap();
        assert_eq!(result, "Hello Alice");
    }

    #[test]
    fn env_interpolation() {
        // SAFETY: test-only, single-threaded test runner
        unsafe { std::env::set_var("AMAKE_TEST_VAR", "42") };
        let result = render(
            "Value: {{env.AMAKE_TEST_VAR}}",
            "test",
            &empty_outputs(),
            &empty_vars(),
            &[],
            &empty_captures(),
        )
        .unwrap();
        assert_eq!(result, "Value: 42");
        unsafe { std::env::remove_var("AMAKE_TEST_VAR") };
    }

    #[test]
    fn task_stdout_interpolation() {
        let mut outputs = BTreeMap::new();
        outputs.insert("build".into(), "build output".into());
        let mut captures = BTreeMap::new();
        captures.insert("build".into(), true);
        let depends = vec!["build".into()];

        let result = render(
            "Build said: {{tasks.build.stdout}}",
            "test",
            &outputs,
            &empty_vars(),
            &depends,
            &captures,
        )
        .unwrap();
        assert_eq!(result, "Build said: build output");
    }

    #[test]
    fn missing_var_errors() {
        let result = render(
            "Hello {{vars.missing}}",
            "test",
            &empty_outputs(),
            &empty_vars(),
            &[],
            &empty_captures(),
        );
        assert!(matches!(result, Err(Error::UnresolvedVariable { .. })));
    }

    #[test]
    fn task_not_in_depends_errors() {
        let mut outputs = BTreeMap::new();
        outputs.insert("build".into(), "output".into());
        let mut captures = BTreeMap::new();
        captures.insert("build".into(), true);

        let result = render(
            "{{tasks.build.stdout}}",
            "test",
            &outputs,
            &empty_vars(),
            &[], // empty depends
            &captures,
        );
        assert!(matches!(result, Err(Error::InvalidTaskReference { .. })));
    }

    #[test]
    fn task_no_capture_errors() {
        let depends = vec!["build".into()];
        let result = render(
            "{{tasks.build.stdout}}",
            "test",
            &empty_outputs(),
            &empty_vars(),
            &depends,
            &empty_captures(), // no capture flag
        );
        assert!(matches!(result, Err(Error::InvalidTaskReference { .. })));
    }

    #[test]
    fn whitespace_in_placeholder() {
        let mut vars = BTreeMap::new();
        vars.insert("x".into(), "yes".into());
        let result = render(
            "{{ vars.x }}",
            "test",
            &empty_outputs(),
            &vars,
            &[],
            &empty_captures(),
        )
        .unwrap();
        assert_eq!(result, "yes");
    }

    #[test]
    fn unknown_namespace_errors() {
        let result = render(
            "{{foo.bar}}",
            "test",
            &empty_outputs(),
            &empty_vars(),
            &[],
            &empty_captures(),
        );
        assert!(matches!(result, Err(Error::UnresolvedVariable { .. })));
    }

    #[test]
    fn multiple_placeholders() {
        let mut vars = BTreeMap::new();
        vars.insert("a".into(), "1".into());
        vars.insert("b".into(), "2".into());
        let result = render(
            "{{vars.a}} and {{vars.b}}",
            "test",
            &empty_outputs(),
            &vars,
            &[],
            &empty_captures(),
        )
        .unwrap();
        assert_eq!(result, "1 and 2");
    }
}
