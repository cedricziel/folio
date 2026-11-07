//! Taxonomy mapping: which Folio tag, correspondent, type or field each
//! Paperless one becomes, and what has to be created or renamed first
//!
//! Plans are deterministic: entities are visited in Paperless ID order, so a
//! dry run and a real run, or two runs over the same library, agree on every
//! name.

use std::collections::HashMap;

use serde::Serialize;

use super::SyncMode;
use crate::models::Id;
use crate::ports::{PaperlessNamed, PaperlessTag};
use crate::services::library::non_empty;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxonomyKind {
    Tag,
    Correspondent,
    DocumentType,
}

impl TaxonomyKind {
    fn label(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Correspondent => "correspondent",
            Self::DocumentType => "document type",
        }
    }
}

/// A live tag, correspondent or type already in the vault
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingEntity {
    pub id: Id,
    pub name: String,
}

/// A `paperless_taxonomy` row: the entity a Paperless one was mapped to, and
/// the name it had at the last sync
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxonomyMapping {
    pub paperless_id: i64,
    pub entity_id: Id,
    pub synced_name: String,
}

/// Where a Paperless entity lands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Target {
    Existing(Id),
    /// Index into [`KindPlan::creates`]
    New(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappedEntity {
    pub paperless_id: i64,
    /// The Folio name this sync wants it to have
    pub name: String,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewEntity {
    pub name: String,
    /// Tags only
    pub color: Option<String>,
}

/// A mirrored Paperless rename of an already mapped entity
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rename {
    pub paperless_id: i64,
    pub entity_id: Id,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaxonomyWarning {
    /// The Paperless name is blank; the entity isn't imported
    InvalidName {
        kind: TaxonomyKind,
        paperless_id: i64,
    },
    /// The Paperless rename was skipped: another live entity has the new name
    RenameCollision {
        kind: TaxonomyKind,
        paperless_id: i64,
        entity_id: Id,
        from: String,
        to: String,
    },
}

/// A same-name tag under a different parent, imported under another name
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlattenedTag {
    pub paperless_id: i64,
    pub name: String,
    pub imported_as: String,
}

/// The plan for one taxonomy kind
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KindPlan {
    /// In Paperless ID order
    pub entities: Vec<MappedEntity>,
    pub creates: Vec<NewEntity>,
    pub renames: Vec<Rename>,
    pub warnings: Vec<TaxonomyWarning>,
}

impl KindPlan {
    pub fn target(&self, paperless_id: i64) -> Option<Target> {
        self.entities
            .binary_search_by_key(&paperless_id, |e| e.paperless_id)
            .ok()
            .map(|i| self.entities[i].target)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TagPlan {
    #[serde(flatten)]
    pub plan: KindPlan,
    /// Paperless IDs of inbox tags; they drive the review state instead
    pub inbox_tags: Vec<i64>,
    pub flattened: Vec<FlattenedTag>,
}

/// The vault's existing state for one taxonomy kind
#[derive(Debug, Clone, Copy)]
pub struct VaultTaxonomy<'a> {
    pub existing: &'a [ExistingEntity],
    pub mappings: &'a [TaxonomyMapping],
}

/// A flattened `<parent>/<name>` longer than this falls back to `<name> (#<id>)`
pub const MAX_FLATTENED_NAME: usize = 128;

/// Flattens tags and maps them. Inbox tags are left out.
///
/// The lowest Paperless ID keeps a name. A later tag with the same name under
/// the same parent is the same tag to Folio; under another parent it becomes
/// `<parent>/<name>`, or `<name> (#<id>)` when it has no parent, the parent
/// is missing, or that name is taken or too long.
pub fn plan_tags(tags: &[PaperlessTag], vault: VaultTaxonomy<'_>, mode: SyncMode) -> TagPlan {
    let mut sorted: Vec<&PaperlessTag> = tags.iter().collect();
    sorted.sort_by_key(|t| t.id);
    let by_id: HashMap<i64, &PaperlessTag> = sorted.iter().map(|t| (t.id, *t)).collect();

    // Chosen names, and the parent of the tag that first took each
    let mut used: HashMap<String, Option<i64>> = HashMap::new();
    let mut flattened = Vec::new();
    let mut inbox_tags = Vec::new();
    let mut warnings = Vec::new();
    let mut wanted = Vec::new();
    for tag in sorted {
        if tag.is_inbox_tag {
            inbox_tags.push(tag.id);
            continue;
        }
        let Some(name) = valid_name(TaxonomyKind::Tag, tag.id, &tag.name, &mut warnings) else {
            continue;
        };
        let chosen = match used.get(name) {
            None => name.to_owned(),
            Some(parent) if *parent == tag.parent => name.to_owned(),
            Some(_) => {
                let under_parent = tag
                    .parent
                    .and_then(|id| by_id.get(&id))
                    .map(|parent| parent.name.trim())
                    .filter(|parent| !parent.is_empty())
                    .map(|parent| format!("{parent}/{name}"))
                    .filter(|n| n.chars().count() <= MAX_FLATTENED_NAME && !used.contains_key(n));
                let imported_as = under_parent.unwrap_or_else(|| format!("{name} (#{})", tag.id));
                flattened.push(FlattenedTag {
                    paperless_id: tag.id,
                    name: name.to_owned(),
                    imported_as: imported_as.clone(),
                });
                imported_as
            }
        };
        used.entry(chosen.clone()).or_insert(tag.parent);
        wanted.push((tag.id, chosen, Some(tag.color.clone())));
    }

    let mut plan = plan_kind(TaxonomyKind::Tag, wanted, vault, mode);
    warnings.append(&mut plan.warnings);
    plan.warnings = warnings;
    TagPlan {
        plan,
        inbox_tags,
        flattened,
    }
}

/// Maps correspondents or document types by name
pub fn plan_named(
    kind: TaxonomyKind,
    entities: &[PaperlessNamed],
    vault: VaultTaxonomy<'_>,
    mode: SyncMode,
) -> KindPlan {
    let mut sorted: Vec<&PaperlessNamed> = entities.iter().collect();
    sorted.sort_by_key(|e| e.id);
    let mut warnings = Vec::new();
    let wanted = sorted
        .into_iter()
        .filter_map(|e| {
            let name = valid_name(kind, e.id, &e.name, &mut warnings)?;
            Some((e.id, name.to_owned(), None))
        })
        .collect();
    let mut plan = plan_kind(kind, wanted, vault, mode);
    warnings.append(&mut plan.warnings);
    plan.warnings = warnings;
    plan
}

/// The trimmed name, validated like names entered in Folio; an invalid one
/// leaves the entity unmapped, so documents import without it
fn valid_name<'n>(
    kind: TaxonomyKind,
    paperless_id: i64,
    name: &'n str,
    warnings: &mut Vec<TaxonomyWarning>,
) -> Option<&'n str> {
    match non_empty(name, kind.label()) {
        Ok(name) => Some(name),
        Err(_) => {
            warnings.push(TaxonomyWarning::InvalidName { kind, paperless_id });
            None
        }
    }
}

fn plan_kind(
    kind: TaxonomyKind,
    wanted: Vec<(i64, String, Option<String>)>,
    vault: VaultTaxonomy<'_>,
    mode: SyncMode,
) -> KindPlan {
    let mappings: HashMap<i64, &TaxonomyMapping> =
        vault.mappings.iter().map(|m| (m.paperless_id, m)).collect();
    // Names held by live entities, and by entities this plan creates
    let mut taken: HashMap<String, Target> = vault
        .existing
        .iter()
        .map(|e| (e.name.clone(), Target::Existing(e.id)))
        .collect();
    let current_name: HashMap<Id, &str> = vault
        .existing
        .iter()
        .map(|e| (e.id, e.name.as_str()))
        .collect();

    let mut plan = KindPlan::default();
    for (paperless_id, name, color) in wanted {
        let mapped = mappings
            .get(&paperless_id)
            .filter(|m| current_name.contains_key(&m.entity_id));
        let target = match mapped {
            Some(m) if m.synced_name == name || mode == SyncMode::NewOnly => {
                Target::Existing(m.entity_id)
            }
            Some(m) => {
                let from = current_name[&m.entity_id].to_owned();
                match taken.get(&name) {
                    Some(&Target::Existing(id)) if id == m.entity_id => {}
                    Some(_) => plan.warnings.push(TaxonomyWarning::RenameCollision {
                        kind,
                        paperless_id,
                        entity_id: m.entity_id,
                        from,
                        to: name.clone(),
                    }),
                    None => {
                        taken.remove(&from);
                        taken.insert(name.clone(), Target::Existing(m.entity_id));
                        plan.renames.push(Rename {
                            paperless_id,
                            entity_id: m.entity_id,
                            from,
                            to: name.clone(),
                        });
                    }
                }
                Target::Existing(m.entity_id)
            }
            None => *taken.entry(name.clone()).or_insert_with(|| {
                plan.creates.push(NewEntity {
                    name: name.clone(),
                    color,
                });
                Target::New(plan.creates.len() - 1)
            }),
        };
        plan.entities.push(MappedEntity {
            paperless_id,
            name,
            target,
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: i64, name: &str, parent: Option<i64>) -> PaperlessTag {
        PaperlessTag {
            id,
            name: name.into(),
            color: "#a6cee3".into(),
            is_inbox_tag: false,
            parent,
        }
    }

    fn named(id: i64, name: &str) -> PaperlessNamed {
        PaperlessNamed {
            id,
            name: name.into(),
        }
    }

    fn entity(name: &str) -> ExistingEntity {
        ExistingEntity {
            id: Id::new_v4(),
            name: name.into(),
        }
    }

    /// The mapping a previous sync recorded, under the entity's current name
    fn mapped(paperless_id: i64, entity: &ExistingEntity) -> TaxonomyMapping {
        TaxonomyMapping {
            paperless_id,
            entity_id: entity.id,
            synced_name: entity.name.clone(),
        }
    }

    const EMPTY: VaultTaxonomy<'static> = VaultTaxonomy {
        existing: &[],
        mappings: &[],
    };

    fn names(plan: &KindPlan) -> Vec<(i64, &str)> {
        plan.entities
            .iter()
            .map(|e| (e.paperless_id, e.name.as_str()))
            .collect()
    }

    #[test]
    fn same_name_tags_under_different_parents_keep_the_lowest_id_unprefixed() {
        let tags = [
            tag(1, "Home", None),
            tag(2, "Work", None),
            tag(3, "Taxes", Some(1)),
            tag(4, "Taxes", Some(2)),
        ];

        let plan = plan_tags(&tags, EMPTY, SyncMode::Mirror);

        assert_eq!(
            names(&plan.plan),
            vec![(1, "Home"), (2, "Work"), (3, "Taxes"), (4, "Work/Taxes")]
        );
        assert_eq!(
            plan.flattened,
            vec![FlattenedTag {
                paperless_id: 4,
                name: "Taxes".into(),
                imported_as: "Work/Taxes".into(),
            }]
        );
        assert_eq!(plan.plan.creates.len(), 4);
    }

    #[test]
    fn two_plans_over_the_same_library_choose_the_same_names() {
        let tags = [
            tag(9, "Taxes", Some(2)),
            tag(1, "Home", None),
            tag(3, "Taxes", Some(1)),
            tag(2, "Work", None),
        ];
        let mut reversed = tags.clone();
        reversed.reverse();

        let first = plan_tags(&tags, EMPTY, SyncMode::Mirror);
        let second = plan_tags(&reversed, EMPTY, SyncMode::Mirror);

        assert_eq!(first, second);
        assert_eq!(first.plan.target(9), Some(Target::New(3)));
    }

    #[test]
    fn inbox_tags_are_not_created() {
        let mut inbox = tag(5, "Inbox", None);
        inbox.is_inbox_tag = true;

        let plan = plan_tags(&[inbox, tag(6, "Bills", None)], EMPTY, SyncMode::Mirror);

        assert_eq!(plan.inbox_tags, vec![5]);
        assert_eq!(names(&plan.plan), vec![(6, "Bills")]);
        assert_eq!(plan.plan.target(5), None);
    }

    #[test]
    fn existing_live_entities_are_reused_by_name() {
        let acme = entity("ACME");
        let existing = [acme.clone()];
        let vault = VaultTaxonomy {
            existing: &existing,
            mappings: &[],
        };

        let plan = plan_named(
            TaxonomyKind::Correspondent,
            &[named(1, "ACME"), named(2, "Bank")],
            vault,
            SyncMode::Mirror,
        );

        assert_eq!(plan.target(1), Some(Target::Existing(acme.id)));
        assert_eq!(plan.target(2), Some(Target::New(0)));
        assert_eq!(
            plan.creates,
            vec![NewEntity {
                name: "Bank".into(),
                color: None
            }]
        );
    }

    #[test]
    fn same_name_entities_collapse_into_one() {
        let plan = plan_named(
            TaxonomyKind::DocumentType,
            &[named(1, "Invoice"), named(2, "Invoice")],
            EMPTY,
            SyncMode::Mirror,
        );
        assert_eq!(plan.creates.len(), 1);
        assert_eq!(plan.target(2), Some(Target::New(0)));
    }

    #[test]
    fn names_match_case_sensitively() {
        let existing = [entity("bills")];
        let vault = VaultTaxonomy {
            existing: &existing,
            mappings: &[],
        };
        let plan = plan_tags(&[tag(1, "Bills", None)], vault, SyncMode::Mirror);
        assert_eq!(plan.plan.target(1), Some(Target::New(0)));
    }

    #[test]
    fn a_paperless_rename_renames_the_mapped_entity_in_mirror_mode() {
        let bills = entity("Bills");
        let existing = [bills.clone()];
        let mappings = [mapped(1, &bills)];
        let vault = VaultTaxonomy {
            existing: &existing,
            mappings: &mappings,
        };

        let plan = plan_tags(&[tag(1, "Invoices", None)], vault, SyncMode::Mirror);

        assert_eq!(
            plan.plan.renames,
            vec![Rename {
                paperless_id: 1,
                entity_id: bills.id,
                from: "Bills".into(),
                to: "Invoices".into(),
            }]
        );
        assert_eq!(plan.plan.target(1), Some(Target::Existing(bills.id)));
        assert!(plan.plan.creates.is_empty());

        let ignored = plan_tags(&[tag(1, "Invoices", None)], vault, SyncMode::NewOnly);
        assert!(ignored.plan.renames.is_empty());
    }

    #[test]
    fn a_rename_onto_an_existing_name_is_skipped_with_a_warning() {
        let bills = entity("Bills");
        let invoices = entity("Invoices");
        let existing = [bills.clone(), invoices];
        let mappings = [mapped(1, &bills)];
        let vault = VaultTaxonomy {
            existing: &existing,
            mappings: &mappings,
        };

        let plan = plan_named(
            TaxonomyKind::Correspondent,
            &[named(1, "Invoices")],
            vault,
            SyncMode::Mirror,
        );

        assert!(plan.renames.is_empty());
        assert_eq!(plan.target(1), Some(Target::Existing(bills.id)));
        assert_eq!(
            plan.warnings,
            vec![TaxonomyWarning::RenameCollision {
                kind: TaxonomyKind::Correspondent,
                paperless_id: 1,
                entity_id: bills.id,
                from: "Bills".into(),
                to: "Invoices".into(),
            }]
        );
    }

    #[test]
    fn swapped_renames_hit_the_collision_guard() {
        let a = entity("A");
        let b = entity("B");
        let existing = [a.clone(), b.clone()];
        let mappings = [mapped(1, &a), mapped(2, &b)];
        let vault = VaultTaxonomy {
            existing: &existing,
            mappings: &mappings,
        };

        let plan = plan_tags(
            &[tag(1, "B", None), tag(2, "A", None)],
            vault,
            SyncMode::Mirror,
        );

        assert!(plan.plan.renames.is_empty());
        assert_eq!(plan.plan.warnings.len(), 2);
    }

    #[test]
    fn a_mapping_to_a_deleted_entity_maps_again_by_name() {
        let mappings = [mapped(1, &entity("Bills"))];
        let vault = VaultTaxonomy {
            existing: &[],
            mappings: &mappings,
        };
        let plan = plan_tags(&[tag(1, "Bills", None)], vault, SyncMode::Mirror);
        assert_eq!(plan.plan.target(1), Some(Target::New(0)));
    }

    #[test]
    fn a_root_tag_after_a_same_name_child_keeps_its_own_entity() {
        let tags = [
            tag(2, "Home", None),
            tag(3, "Taxes", Some(2)),
            tag(5, "Taxes", None),
        ];

        let plan = plan_tags(&tags, EMPTY, SyncMode::Mirror);

        assert_eq!(
            names(&plan.plan),
            vec![(2, "Home"), (3, "Taxes"), (5, "Taxes (#5)")]
        );
        assert_ne!(plan.plan.target(3), plan.plan.target(5));
    }

    #[test]
    fn a_missing_parent_or_a_taken_or_long_flattened_name_falls_back_to_the_id() {
        let long = "P".repeat(MAX_FLATTENED_NAME);
        let tags = [
            tag(1, "Home", None),
            tag(2, long.as_str(), None),
            tag(3, "Taxes", Some(1)),
            tag(4, "Taxes", Some(99)),
            tag(5, "Home/Taxes", None),
            tag(6, "Taxes", Some(2)),
            tag(7, "Taxes", Some(5)),
        ];

        let plan = plan_tags(&tags, EMPTY, SyncMode::Mirror);
        let names = names(&plan.plan);

        assert_eq!(names[3], (4, "Taxes (#4)"));
        assert_eq!(names[5], (6, "Taxes (#6)"));
        assert_eq!(names[6], (7, "Home/Taxes/Taxes"));
        let distinct: std::collections::HashSet<_> = names.iter().map(|(_, n)| *n).collect();
        assert_eq!(distinct.len(), names.len());
    }

    #[test]
    fn names_are_trimmed_and_blank_names_are_skipped_with_a_warning() {
        let bills = entity("Bills");
        let existing = [bills.clone()];
        let vault = VaultTaxonomy {
            existing: &existing,
            mappings: &[],
        };

        let plan = plan_named(
            TaxonomyKind::Correspondent,
            &[named(1, " Bills "), named(2, "   ")],
            vault,
            SyncMode::Mirror,
        );

        assert_eq!(plan.target(1), Some(Target::Existing(bills.id)));
        assert_eq!(plan.target(2), None);
        assert_eq!(
            plan.warnings,
            vec![TaxonomyWarning::InvalidName {
                kind: TaxonomyKind::Correspondent,
                paperless_id: 2
            }]
        );

        let tags = plan_tags(&[tag(3, "", None)], EMPTY, SyncMode::Mirror);
        assert!(tags.plan.entities.is_empty());
        assert_eq!(tags.plan.warnings.len(), 1);
    }
}
