//! Which packages can load together, and in which order.

use semver::Version;

use super::manifest::PackageManifest;
use crate::error::EngineError;

/// `packages` ordered so that every package comes after the packages it depends on, keeping the
/// given order where the dependencies allow it.
///
/// # Errors
///
/// Fails, naming the package, on a name given twice, an `engine` requirement `engine` does
/// not meet, a dependency that is missing or in a version that does not match, and a
/// dependency cycle (with the cycle).
pub(super) fn load_order(
    packages: Vec<PackageManifest>,
    engine: &Version,
) -> Result<Vec<PackageManifest>, EngineError> {
    check_each(&packages, engine)?;
    let mut state = vec![Visit::Todo; packages.len()];
    let mut order = Vec::with_capacity(packages.len());
    let mut path = Vec::new();
    for index in 0..packages.len() {
        visit(index, &packages, &mut state, &mut path, &mut order)?;
    }
    let mut slots: Vec<Option<PackageManifest>> = packages.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect())
}

fn fail(manifest: &PackageManifest, details: String) -> EngineError {
    EngineError::Manifest {
        path: manifest.name.clone(),
        details,
    }
}

fn check_each(packages: &[PackageManifest], engine: &Version) -> Result<(), EngineError> {
    for (index, manifest) in packages.iter().enumerate() {
        if let Some(first) = packages[..index]
            .iter()
            .find(|other| other.name == manifest.name)
        {
            return Err(fail(
                manifest,
                format!(
                    "two packages are named `{}`: {} and {}",
                    manifest.name,
                    first.root.display(),
                    manifest.root.display()
                ),
            ));
        }
        if let Some(requires) = &manifest.engine_requires
            && !requires.matches(engine)
        {
            return Err(fail(
                manifest,
                format!("needs Manivelle {requires}, this is {engine}"),
            ));
        }
        for dependency in &manifest.depends {
            let Some(found) = packages.iter().find(|other| other.name == dependency.name) else {
                return Err(fail(
                    manifest,
                    format!("needs package `{}`, which is not there", dependency.name),
                ));
            };
            if !dependency.requirement.matches(&found.version) {
                return Err(fail(
                    manifest,
                    format!(
                        "needs `{}` {}, found {}",
                        dependency.name, dependency.requirement, found.version
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Todo,
    Open,
    Done,
}

/// Depth first: a package is placed once all its dependencies are.
fn visit(
    index: usize,
    packages: &[PackageManifest],
    state: &mut [Visit],
    path: &mut Vec<usize>,
    order: &mut Vec<usize>,
) -> Result<(), EngineError> {
    match state[index] {
        Visit::Done => return Ok(()),
        Visit::Open => {
            let start = path.iter().position(|&seen| seen == index).unwrap_or(0);
            let mut cycle: Vec<&str> = path[start..]
                .iter()
                .map(|&i| packages[i].name.as_str())
                .collect();
            cycle.push(packages[index].name.as_str());
            return Err(fail(
                &packages[index],
                format!("dependency cycle: {}", cycle.join(" -> ")),
            ));
        }
        Visit::Todo => {}
    }
    state[index] = Visit::Open;
    path.push(index);
    for dependency in &packages[index].depends {
        // `check_each` found every dependency.
        if let Some(next) = packages
            .iter()
            .position(|other| other.name == dependency.name)
        {
            visit(next, packages, state, path, order)?;
        }
    }
    path.pop();
    state[index] = Visit::Done;
    order.push(index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn manifest(
        name: &str,
        version: &str,
        depends: &[(&str, &str)],
        engine: Option<&str>,
    ) -> PackageManifest {
        let mut body = format!("package \"{name}\" {{\n version \"{version}\"\n entry \"a.ts\"\n");
        if let Some(engine) = engine {
            body += &format!(" engine {{\n requires \"{engine}\"\n }}\n");
        }
        if !depends.is_empty() {
            body += " depends {\n";
            for (dependency, requirement) in depends {
                body += &format!("  \"{dependency}\" version \"{requirement}\"\n");
            }
            body += " }\n";
        }
        body += "}\n";
        PackageManifest::parse(&body, Path::new(&format!("/packages/{name}"))).unwrap()
    }

    fn names(order: &[PackageManifest]) -> Vec<&str> {
        order.iter().map(|m| m.name.as_str()).collect()
    }

    fn engine() -> Version {
        Version::new(0, 2, 0)
    }

    fn error(packages: Vec<PackageManifest>) -> String {
        load_order(packages, &engine()).unwrap_err().to_string()
    }

    #[test]
    fn dependencies_load_first_and_the_given_order_breaks_ties() {
        let order = load_order(
            vec![
                manifest("combat", "1.0.0", &[("core", "^1.0"), ("ui", "*")], None),
                manifest("ui", "0.3.0", &[("core", ">=1")], None),
                manifest("core", "1.4.0", &[], None),
                manifest("extra", "1.0.0", &[], None),
            ],
            &engine(),
        )
        .unwrap();

        assert_eq!(names(&order), ["core", "ui", "combat", "extra"]);
    }

    #[test]
    fn refuses_what_cannot_load_together() {
        let missing = error(vec![manifest("a", "1.0.0", &[("b", "*")], None)]);
        assert!(missing.contains("needs package `b`"), "{missing}");

        let mismatch = error(vec![
            manifest("a", "1.0.0", &[("b", "^2.0")], None),
            manifest("b", "1.5.0", &[], None),
        ]);
        assert!(
            mismatch.contains("needs `b` ^2.0, found 1.5.0"),
            "{mismatch}"
        );

        let engine_too_old = error(vec![manifest("a", "1.0.0", &[], Some(">=0.4.0"))]);
        assert!(
            engine_too_old.contains("needs Manivelle >=0.4.0, this is 0.2.0"),
            "{engine_too_old}"
        );

        let twice = error(vec![
            manifest("a", "1.0.0", &[], None),
            manifest("a", "2.0.0", &[], None),
        ]);
        assert!(twice.contains("two packages are named `a`"), "{twice}");
    }

    #[test]
    fn a_dependency_cycle_is_reported_with_its_path() {
        let cycle = error(vec![
            manifest("a", "1.0.0", &[("b", "*")], None),
            manifest("b", "1.0.0", &[("c", "*")], None),
            manifest("c", "1.0.0", &[("a", "*")], None),
        ]);
        assert!(cycle.contains("a -> b -> c -> a"), "{cycle}");
    }
}
