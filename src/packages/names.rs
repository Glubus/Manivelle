//! Matching permission names against contract names, and suggesting the near miss of a typo.

use super::PermissionSet;

/// Whether `permission` (a name, `namespace.*` or `*`) covers at least one of `contracts`.
pub(crate) fn permission_matches(permission: &str, contracts: &[&String]) -> bool {
    PermissionSet::new(&[permission.to_owned()])
        .allows_any(contracts.iter().map(|name| name.as_str()))
}

/// The candidate closest to `name`, when it is a plausible typo of it.
pub(crate) fn closest<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    candidates
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

/// The number of single-character edits that turn `a` into `b`.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b_len = b.chars().count();
    let mut row: Vec<usize> = (0..=b_len).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if ca == cb {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[j])
            };
            diagonal = above;
        }
    }
    row[b_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typo_gets_the_closest_candidate() {
        let candidates = ["world.modify", "world.read", "entity.spawn"];
        assert_eq!(
            closest("world.modfy", candidates.iter().copied()),
            Some("world.modify")
        );
        assert_eq!(closest("something.else", candidates.iter().copied()), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }
}
