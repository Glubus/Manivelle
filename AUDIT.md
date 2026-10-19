# Audit du code Manivelle

Audit en lecture seule : **aucun fichier de code n'a été modifié**. Chaque point a un ID (`B1`, `S2`…) pour que tu puisses répondre « fais B1, S2, ignore D3 ».

- Sévérité : **H** = bug probable / à faire d'abord, **M** = vaut le coup, **L** = polish.
- Méthode : lecture du code, `clippy` pedantic/nursery (bruit non repris, voir §10), scan de longueur des fonctions, 4 relectures parallèles par zone. Les points marqués ✔ ont été revérifiés à la main ; les autres viennent des relectures et sont sourcés par `fichier:ligne`.
- Base : `cargo clippy --all-targets` standard est propre (0 warning). Le code est globalement sain : peu de `unsafe` hors FFI QuickJS, fonctions courtes (seulement 3 > 80 lignes), pas de TODO/FIXME/`dbg!`.

---

## 0. Résumé exécutif

| Zone | État | Gros chantiers |
|---|---|---|
| Générateurs TS (`registry/host_contracts`) | Duplication forte | `declarations.rs` ≈ `sdk.rs` ; deux arbres de noms ; deux listes de mots réservés qui divergent (bug) |
| API du registre | Redondante | trait `HostContractRegistry` à 1 seule impl + méthodes inhérentes qui re-wrappent |
| `Engine` / `lifecycle.rs` | God struct / fichier fourre-tout | `Engine` 27 champs, `lifecycle.rs` 900 lignes |
| `contract/` | Un module qui n'a pas sa place | `host_resolver.rs` = plomberie runtime ; validation JSON via `serde_json::Value` (perf) |
| CLI (`bin/manivelle.rs`) | Fourre-tout | parsing + scaffolding + run + sdk ; nom `run` en double |
| Tests | Fichiers énormes, helpers copiés 15× | `tests/derive_schema.rs` 1157 lignes ; `fn engine()` ×15 |

Bugs réels trouvés (à traiter avant le refactor) : **B1–B6**.

---

## 1. Bugs probables / correctness (à faire d'abord)

- **B1 [H] ✔** `src/registry/host_contracts/import_modules.rs:~186` — la liste de `is_reserved_word` (lue en entier) ne contient pas `function`, `enum`, `null`, `true`, `false`, `let`, `static`, `await`. Un export nommé `function` ou `null` génère `export const function = …` → SyntaxError dans le module généré. `sdk/ident.rs:23-60` a sa propre liste (avec `function`) → les deux divergent. *Fix : une seule liste partagée.*
- **B2 [H]** `import_modules.rs:~150` — `identifier()` replie tout nom invalide (`my-lib`) sur `"sdk"` ; deux contrats invalides → deux `export const sdk` → SyntaxError. *Fix : erreur explicite ou suffixe unique.*
- **B3 [H] ✔** `src/packages/manifest.rs:434` — `line_of` utilise `source.chars().take(offset)` alors que `offset` (span KDL) est en **octets** → ligne fausse dès qu'il y a du non-ASCII avant l'erreur. Quadratique aussi (recompté à chaque nœud). *Fix : `compiler/diagnostics.rs::line_column` existe déjà ; index de débuts de ligne.*
- **B4 [H] ✔** `src/runner/quota.rs:80-82` — `u32::try_from(index+1).expect("fits in 24 bits")` ne vérifie que 32 bits ; la vraie borne (`MAX_OWNERS`, 24 bits de header) n'est vérifiée que par `debug_assert!` après coup. En release → header corrompu. *Fix : `assert!(index < MAX_OWNERS)` avant la conversion.*
- **B5 [M]** `src/registry/host_contracts/bindings.rs:394` et `memory.rs:323` — `HashMap::insert` écrase silencieusement un contrat enregistré 2× sous le même nom. De plus le descripteur est inséré *avant* la liaison : si la 2e étape échoue le registre est incohérent. *Fix : erreur sur doublon ; insérer les deux sous un seul verrou.*
- **B6 [M]** `src/contract/codec/field_atoms.rs:54-70` — fuite d'atome QuickJS si `store_userdata` échoue (atome créé non libéré avant `Err`).
- **B7 [M]** `src/runner/engine/{delivery.rs:158-169, host_driven.rs:227-241,267-285}` — `return` silencieux quand `get_index` échoue : abandonne les scripts restants au lieu de continuer.
- **B8 [M]** `src/runner/promise_rejections.rs:54-65` — `take` vide toute la liste mais ne renvoie que la première rejection ; les autres sont perdues sans trace.
- **B9 [M]** `src/contract/validation.rs:410` — `validate_union` jette l'erreur de chaque membre : message final « did not match any union member » sans le détail utile.
- **B10 [L]** `src/registry/host_contracts/import_modules.rs:71` — `insert_binding` ignore un chemin vide en silence.
- **B11 [L]** `src/runner/retained.rs:146`, `contract/codec/field_atoms.rs:80` — `name.len() as _` (usize→c_int) sans contrôle de troncature.
- **B12 [L]** `src/bin/manivelle/run.rs:142` — `new_token` fabrique un secret avec `RandomState` ×4 + horloge + pid hashés en blake3 ; pas un CSPRNG. *Fix : `getrandom`.*
- **B13 [L]** `src/analysis/usage.rs:374` — `is_generated` détecte le SDK généré via `source.contains("function __hostCall")` : couplage fragile avec `sdk.rs:781`. *Fix : marqueur/constante partagé.*

---

## 2. Duplication

- **D1 [H] ✔** `registry/host_contracts/declarations.rs` vs `sdk.rs` — `DeclarationBuffer` et `SdkBuilder` ont le même `push_descriptor` (match sur `HostContractAbi` identique, octet pour octet), les mêmes `push_function/callback/context/export`, le même squelette `finish`. *Fix : un trait/visiteur `DescriptorSink` (ou un seul parcours qui produit un modèle intermédiaire `ContractModel` dont `declarations` et `sdk` sont deux rendus).*
- **D2 [M]** `import_modules.rs:70-200` vs `sdk/tree.rs`/`sdk/ident.rs` — deux arbres de noms quasi identiques (`HostModuleNode` vs `ObjectTree`), `identifier`/`property_name`/`is_identifier`/`indent` copiés. *Fix : `NameTree<T>` générique + `sdk/ident.rs` seul.*
- **D3 [M]** `registry/host_contracts/memory.rs` — `function_descriptor`, `callback_descriptor`, `register_context`, `script_export` construisent chacun `schema = …; abi = …` ; `script_export` réimplémente le verrou au lieu d'appeler `insert_descriptor`.
- **D4 [M]** `bindings.rs:~170-260` — `PlainBinding::install` / `CallerBinding::install` : même `match validator` + `start_clock`. `ValidatedReply::into_js` ré-implémente `check_output`.
- **D5 [M]** `runner/engine/dispatch.rs:32` + `host_driven.rs:77` — même prologue (`charge_to`, `budget`, `context.with`, `settle`, `attribute_interrupt`). *Fix : `run_in_script(&self, script, f)`.* `emit_report` (dispatch.rs:98) refait à la main ce que `attribute_interrupt` fait déjà.
- **D6 [M]** `runner/engine/faults.rs:194` vs `213` — « pool presque plein » (`malloc_size*10 >= limit*9`) écrit 2×. `faults.rs:256-283` (`drain_jobs`) vs `dispatch.rs:216` (`settle`) : deux boucles de drainage des jobs.
- **D7 [M]** `runner/tasks.rs:118` vs `191` — même closure `decode_js(...).catch(ctx).map_err(caught_js_error)`.
- **D8 [M]** `runner/engine/lifecycle.rs:888` vs `runner/web.rs:108` — même correctif de prototype de contexte, 2 implémentations.
- **D9 [M]** `analysis/check.rs:124,145,160,181` — suggestion « did you mean » construite 3 fois. Comptage de lignes en 4 endroits (`manifest.rs:434`, `usage.rs:131`, `check.rs:333/343/363`) alors que `compiler/diagnostics.rs:56 line_column` existe.
- **D10 [M]** `bin/manivelle/check.rs:44` vs `run.rs:121` — boucle canonicalize + dédoublonnage des racines de packages identique. *Fix : `packages::canonical_roots`.*
- **D11 [M]** macros : trois `syn::meta::parser` artisanaux (host_fn.rs:277, host_mod.rs:38, script_export.rs:56) ; `Kind` matché à 4 endroits dans host_fn.rs ; `scope`/`variant_scope` quasi identiques (struct_codec.rs:38 / enum_codec.rs:353).
- **D12 [L]** `contract/validation.rs:427` — `validate_optional` ≡ `validate_nullable`. `codec/maps.rs:154` (`is_dropped_by_json`) ≡ `derive.rs:70` (`is_absent`). `codec/number.rs:96` et `:195` — même règle entier/`-0.0` écrite 2×. `bridge.rs:157 bytes_snapshot` vs `codec/bytes.rs` — lecture unsafe des octets dupliquée.
- **D13 [L]** `runner/timers.rs:27-75` — `NextTimer` et `EarliestDue` : deux `Rc<Cell<f64>>` quasi identiques. `release_value` répété (`timers.rs:115`, `events.rs:244`). Motif `JS_IsException`/`pending_exception` répété (`retained.rs`, `engine/mod.rs:364`, `invoke.rs:39`).
- **D14 [L]** `analysis/check.rs:117` vs `packages/manifest.rs:248,266` — validation de nom dupliquée ; `usage.rs:191` vs `:374` — suffixes `.sdk*` dupliqués ; `compiler/resolver.rs:165` contourne `parent_dir`.
- **D15 [L]** `sdk_files.rs::write_host_sdk_files*` vs `InMemoryHostContractRegistry::write_sdk_files*` : 2 chemins pour la même chose (+ `export_sdk`).

---

## 3. Fonctions trop longues / trop de responsabilités (à découper)

Les 3 plus longues (> 80 lignes) ; tout le reste est < 80.

| Fonction | Lignes | Découpe proposée |
|---|---|---|
| `packages/resolve.rs:140 manifest` | 86 | lecture → validation → construction |
| `analysis/check.rs:133 check_usage` | 83 | une fonction générique par catégorie (fonctions / événements) + `suggestion_for` |
| `compiler/project.rs:91 discover_project` | 79 | `reuse_state` + `collect_modules` |
| `macros/host_fn.rs:174 signature` | 70 | extraction des types / génération par `Kind` ; supprimer l'indexation `types[len-1]` (panic de proc-macro si l'invariant casse) |
| `protocol/link.rs:62 connect` | 66 | handshake / threads |
| `protocol/mod.rs:79 setup_protocol` | ~44 mais 5 rôles | export, connexion, tap console, packages, reporting → étapes nommées ; `run_request` bras `Reload` → `reload_reply` |
| `runner/engine/lifecycle.rs:406 mount` | 54 | budget + quota + groupe + promesses + rollback ; `roll_back_mount(group.filter(\|_\| created_group))` est illisible → `target()` renvoie `created_group` |
| `bin/manivelle/run.rs:56 run_target` | 47 | spawn / attente / mode attended / sortie |
| `analysis/check.rs:255 add_permissions` | 46 | 3 branches de blocs KDL quasi identiques → constructeur de bloc (et déplacer vers `packages`) |
| `macros/attrs.rs:269 SerdeAttrs::apply` | 49 | acceptable ; factoriser `push_json_only` |
| `runner/events.rs:208 deliver/call_handler` | — | `keep_results: bool` + closure `on_return` → enum `Delivery { Collect, Discard }` |

**Fichiers/structs trop gros :**

- **G1 [H]** `runner/engine/lifecycle.rs` (900 l) mélange : API de chargement (`load_script*`, `load_project*`), hot-reload (`reload_changed`, `restamp`, `watch_failed_attempt`), montage (`mount*`), prélude/web APIs (`run_prelude`, `declare_*`, `web_global_names`), construction d'env (`build_env`, `env_hooks`, `host_functions`), création de contexte (`new_context`). *Fix : `engine/load.rs`, `engine/hot_reload.rs`, `engine/mount.rs`, `engine/prelude.rs`, `engine/context.rs`.*
- **G2 [H]** `Engine` : 27 champs (`runner/engine/mod.rs:78`). Regrouper : `Scheduling { timer_clock, earliest_timer, host_wake, active_tasks }`, `Faults { max_script_faults, memory_fault_seen, quota }`, `Prelude { prelude, host_class, web_bytecode, export_atoms }`, `Packages { packages, permissions }`. Idem `EngineScript` (15 champs, construit à la main dans `mount_graph`, lifecycle.rs:344).
- **G3 [M]** `registry/host_contracts/sdk.rs` (997 l) = builder + rendu des *model helpers/prédicats runtime* (`render_model_*`, `render_*_predicate`) + rendu d'arbres. *Fix : `sdk/{builder,models,predicates,tree_render}.rs`.*
- **G4 [M]** `bin/manivelle.rs` (413 l) : CLI clap + `init` + `run_program`/`wait_or_kill` + `scaffold_project`/`package_manifest` (KDL via `format!`) + `replace_file`/`create_if_missing` + `sdk`. *Fix : `bin/manivelle/{cli,init,sdk}.rs`.* Nom `fn run` + `mod run` source de confusion → `dispatch`/`execute`.
- **G5 [M]** `packages/mod.rs` : types publics + `Engine::load_packages` + `resolve_packages`/`discover` ; `packages/manifest.rs` (591 l) réimplémente une couche `Node`/`Failure`/`Fields` par-dessus `kdl`.
- **G6 [M]** `manivelle_macros/src/attrs.rs` (454 l) : parsing serde + parsing manivelle + structs container/field/variant → 2 modules.
- **G7 [L]** `Engine::load_script` / `load_script_in` et `load_project` / `load_project_in` : paires de wrappers quasi identiques, avec `self.permissions.remove(&id)` répété.
- **G8 [L]** `runner/errors.rs:74-132` : mapping d'erreurs + réécriture de localisations TS → `runner/source_locations.rs`.

---

## 4. Non-idiomatique Rust

- **I1 [M]** Types de domaine « stringly » : `EngineError::ContractValidation.direction: &'static str` (`"input"`/`"output"`) → `enum Direction` ; `codec_error(from: &'static str, expected: &'static str, …)` idem ; `ScriptId = String` alias faible → newtype ; `EngineError::{Manifest, ScriptNotFound, FunctionNotFound}` en `String`.
- **I2 [M]** Sentinelles magiques : `Schema::named("unknown")` + `HostContractAbi::Unknown` comparé par chaîne (`schema.rs` `has_declaration_name_str`) ; `NAN_STAND_IN = u64::MAX` etc. dans `bridge.rs:18` → `enum NonFinite` ; `NO_DEADLINE`/`PENDING = u64::MAX-1` dans `execution.rs:85` → enum/`Option`.
- **I3 [M]** `contract/codec.rs:180` (`prefix_path`/`has_path`) détecte un préfixe de chemin en parsant le texte du message (`starts_with('[')`) → porter le chemin comme donnée dans `JsError`.
- **I4 [M]** `bindings.rs:319 js_host_error` : écrase toute `EngineError` en `Error::new_from_js_message("host","function",…)` → perd la variante source.
- **I5 [L]** `Result<(), ()>` dans `protocol/link.rs:43,131` → type d'erreur dédié.
- **I6 [L]** Petits idiomes : `ready.front().is_some()` → `!is_empty()` (host_resolver.rs:68) ; `impl<'a> ValidationContext<'a>` → `'_` ; `.map(..).unwrap_or(false)` → `is_ok_and` (validation.rs) ; `segments.remove(0)` (O(n)) → `split_first` (`sdk/tree.rs:61`) ; `#[allow(clippy::float_cmp)]` sur une instruction (number.rs:100) → niveau fonction ; `#[derive(Debug)]` au-dessus de la doc (`compiler/resolver.rs:24`) ; doc sur `fn drop` (`promise_rejections.rs:68`).
- **I7 [L]** `imports_of: &'a mut ImportsOf<'a>` (`compiler/project.rs:174`) force l'invariance → `&'a mut ImportsOf<'b>`.
- **I8 [L]** Récursion non bornée sur la chaîne de dépendances (`packages/resolve.rs:86`) ; `position(..).unwrap_or(0)` (`resolve.rs:104`) masque un invariant.
- **I9 [L]** `HostFnArgs::parse(args.clone())` clone un `TokenStream` inutile (`macros/host_fn.rs:51`, `host_event.rs:12`).
- **I10 [L]** `expect()` en lib : `attrs.rs:108`, `collections.rs:113`, `stack_text.rs:24`, `quota.rs:80`, `retained.rs:55`, `tasks.rs:169` — documenter l'invariant ou convertir en erreur.
- **I11 [L]** `contract/validation.rs`, `check.rs` : erreurs en `String` brute (`Result<(), String>`) → type d'erreur avec chemin structuré.

---

## 5. Noms / API publique

- **N1 [M]** `InMemoryHostContractRegistry::descriptor(name)` vs `HostContract::descriptor()` : même nom, sens différents. `MapKey` (`codec/maps.rs`) vs `TsRecordKey` (`metadata.rs`) : deux concepts de clé de map sans lien.
- **N2 [M]** Trait `HostContractRegistry` : **une seule implémentation**, et `InMemoryHostContractRegistry` expose en plus des méthodes inhérentes **de noms différents** (`function` / `register_function`, `function_with` / `register_function_with`, …) qui appellent le trait. 3 surfaces pour le même geste (trait, inhérent, macros `host_fn`). *Fix : supprimer le trait (ou l'inhérent) ; garder une seule famille de noms.* C'est un changement d'API publique (semver).
- **N3 [L]** `HostRequest` (callback + réponse) vs `HostCallback` : prête à confusion ; `HostResolver`/`ReplyInbox`/`Settlement`/`EncodeReply` mélangent public et interne.
- **N4 [L]** `dispatch` (`Engine::dispatch`) vs `events::deliver` : vocabulaire surchargé ; `NextTimer` vs `EarliestDue` (cf. D13) ; `bridge.rs` : `js_value_to_json` / `json_to_js_value` / `js_value_to_validation_snapshot` non homogènes ; `codec_error` (pub, hidden) vs `encode_error` (pub(crate)) ; `expect_array/expect_array_len/expect_object` visibilités incohérentes.
- **N5 [L]** `context_run_end(…, |index| index)` (`delivery.rs`) : closure d'indexation inutile → `run_end(start, count)`. `render_model_class(name, _ty)` / `render_model_helper_entry(name, _ty, …)` : paramètre `_ty` inutilisé. `model.rs:106 rust_name` = simple `to_string`.
- **N6 [L]** Surface publique de `lib.rs` : 9 re-exports `__xxx` `#[doc(hidden)]` éparpillés → un seul `#[doc(hidden)] pub mod __private`. `JsEncode::encode_scalar` / `JsDecode::decode_scalar` (hidden mais méthodes publiques du trait) → sealed/`pub(crate)`. `contract/codec/derive.rs` est un `mod` public juste pour la sortie des macros. `src/js.rs` réexporte `String`/`Result`/`Error` de rquickjs : `js::String` masque `std::string::String` chez l'utilisateur. `PackageFailure` dérive `Serialize` alors que `PackageInfo` non (incohérent).
- **N7 [L]** Terminologie obsolète : test `rejects_entries_that_leave_the_mod_directory` (« mod » → « package »). `contract/caller.rs:6-8` : le même lien doc répété deux fois (le 2ᵉ vise sans doute `async_function_with_caller`). Doc de `WatchedFiles::merge` (`stamp.rs:61`) dit « replacing » mais le code fait `extend`.
- **N8 [L]** Visibilités incohérentes : `pub` dans module `pub(super)` (`tasks.rs:48,54,182`) ; champs `pub(crate)` de `Engine` avec chemins `crate::packages::…` en ligne au lieu d'`use`. (clippy `redundant_pub_crate` en signale ~150 ; pas à faire en masse.)

---

## 6. Modules mal placés / obsolètes

- **M1 [M]** `contract/host_resolver.rs` : contient la machine d'état runtime (`ReplyInbox`, `InboxState`, `Settlement`) → `runner/host_promises`. `contract/` devrait ne garder que les types de contrat.
- **M2 [M]** `analysis/check.rs` héberge des utilitaires génériques (`permission_matches`, `closest`, `edit_distance`) et l'édition de KDL (`add_permissions`) → utilitaire + `packages`.
- **M3 [L]** `runner/engine/host_driven.rs::harvest_tasks` est appelé par `dispatch.rs::settle` → vit avec `settle`.
- **M4 [L]** `sdk_files.rs`, `types.rs`, `js.rs` : petits, cohérents ; `sdk_files` à fusionner avec le registre (cf. D15). Aucun module vide ni mort trouvé : `cache/`, `compiler/`, `protocol/`, `analysis/` sont tous utilisés.
- **M5 [L]** `compiler/source_map.rs:68` `decode` : `sources: vec![String::new()]` en dur.
- **M6 [L]** `bindings.rs:~415 mod lock_tests` dans le fichier de prod alors que `host_contracts/tests.rs` existe.

---

## 7. Performance

- **P1 [H]** `contract/bridge.rs:157 bytes_snapshot` : chaque `Uint8Array`/`ArrayBuffer` validé devient un `Vec<serde_json::Value::Number>` (un `Value` par octet, ~×16 mémoire + 1 alloc/octet). *Fix : valider sur `&[u8]` sans snapshot JSON.*
- **P2 [M]** `contract/validation.rs:231-294` : `child_path()` (`format!`) alloué pour chaque champ même valide ; `resolving_type_refs: Vec<String>` clone à chaque `TypeRef`, `find_schema_dependency` linéaire. *Fix : chemin paresseux / segments empruntés, index résolu une fois.*
- **P3 [M]** `runner/engine/host_driven.rs:107 pump()` itère tous les scripts dès que `host_wake` est levé ; `request_deferred`/`poll_tasks` font `retain` sur tous les scripts à chaque appel. `delivery.rs:237 encoding_failed` clone `details: String` pour chaque script écoutant.
- **P4 [M]** `packages/manifest.rs:434` + `analysis/usage.rs:131` : comptage de lignes quadratique (cf. B3). `packages/resolve.rs:41-125` : `find` linéaire sur les dépendances (O(n²)) → `HashMap` nom→index.
- **P5 [M]** `compiler/project.rs:146-154,225` : clone de `module.source`, `display_path`, `resolved_requests`, `dependencies` par module à chaque découverte → emprunter / `Arc`.
- **P6 [L]** `registry` : `descriptors()`/`list()` clone + trie tous les descripteurs à chaque `import_modules`/`sdk` ; `bindings.rs::snapshot` clone toute la `HashMap` (noms inclus) à chaque installation de script → `Arc<[..]>` ou itérer sous verrou.
- **P7 [L]** `contract/codec/number.rs:206 widen_f32` : formatage texte + reparse pour chaque `f32` non entier (sert à garder la forme décimale la plus courte → à justifier en commentaire ou remplacer par `f64::from`, **change la valeur observée**, donc à trancher).
- **P8 [L]** `runner/promise_rejections.rs:76 Rejection::is` clone un `Persistent` par comparaison ; `events.rs:67 ListenedEvents::handlers` recherche linéaire ; `events.rs:179 EventIndex::rebuild` réalloue des `Box<str>` ; `Engine.active_tasks` compteur redondant avec `tasks`/`request_guards` (risque de désynchronisation) ; `import_modules.rs:96` strings imbriquées par niveau → `&mut String`.
- **P9 [L]** `analysis/check.rs:236 edit_distance` alloue un `Vec<char>` par appel ; `check.rs:195` boucle permissions × callables avec scan `Vec`. `cache/store.rs:41,76` copie le JS en `String` à chaque écriture. `manifest.rs:345` `one_arg` renvoie une `String` clonée.

---

## 8. `unsafe` : documentation / robustesse

~70 blocs `unsafe`, tous dans la couche FFI QuickJS (`runner/{host_fn,quota,retained,events,timers,web}.rs`, `engine/*`, `contract/codec/*`). `#![warn(clippy::undocumented_unsafe_blocks)]` est actif, donc tout est commenté ; les points ci-dessous sont des invariants **non vérifiables localement** :

- **S1 [M]** `quota.rs:248-301` — `Layout::from_size_align_unchecked(HEADER+size, ALIGN)` repose sur le cap `SIZE_MASK` sans assertion locale → un helper `layout_for(size)` avec `debug_assert`. `quota.rs:98-114` `credit` : `used.get() - bytes` non vérifié (wrap en release), `accounts[owner-1]` non borné. `read_header` (`quota.rs:225`) sans SAFETY détaillé.
- **S2 [M]** `host_fn.rs:135` `&*JS_GetOpaque(..)` sans test de null ; `host_fn.rs:167` `NonNull::new_unchecked(ctx)` sans preuve locale → `NonNull::new` ou SAFETY sur l'origine.
- **S3 [M]** `web.rs:80` `bytes.as_bytes()` : le SAFETY dit « copié avant que du JS tourne » mais ça n'est pas imposé par le type → copier d'abord.
- **S4 [L]** `engine/mod.rs:364` : commentaire SAFETY placé au-dessus du mauvais bloc (le `unsafe` est dans la closure). `lifecycle.rs:826 new_context` : `enabled && unsafe { add(raw) } != 0` peu lisible → tableau `(bool, fn)` + boucle.

---

## 9. Tests, benches, exemples, config

- **T1 [H]** `tests/derive_schema.rs` (1157 l, 33 tests, 6 sujets, 34 types annotés `#[allow(dead_code)]`) → 4 fichiers par thème + un module de types avec un seul `allow` justifié.
- **T2 [M]** Gros fichiers à scinder par thème : `engine.rs` (834), `engine_projects.rs` (828), `type_contract_derive.rs` (716), `context_groups.rs` (699) ; en `src/` : `runner/host_promises.rs` et `runner/execution.rs` ont ~200 lignes de tests inline.
- **T3 [M]** Helpers de test copiés : `fn engine()` dans ~15 fichiers (`builtins`, `call_scalars`, `context_groups`, `contract_validation`, `engine`, `engine_projects`, `host_functions`, `hot_state`, `package_permissions`, `packages`, `requests`, `script_export`, `script_globals`, `source_maps`, `interrupt`) ; `assert_tsc` ×2 (`requests.rs:258`, `host_promises.rs:517`) + équivalents dans `hot_state.rs`; `run_tsc`/`run_tsc_project` copiés dans `support/mod.rs:8-50` ; `write`/`write_file` ×3 ; `count` ×2 ; `stdout`/`stderr` ×3 ; 3 writers de tsconfig. → tout dans `tests/support/`.
- **T4 [M]** Tests qui épinglent le texte généré (conflit avec la règle « pas de tests de wording ») : `host_registry_api.rs:173-185` (dts entier), `:199-245` (~15 `contains`), `dogfood_usage.rs:174` (ligne TS exacte), `schema.rs:331-447` (messages d'erreur textuels). → vérifier par `tsc`/structure.
- **T5 [M]** `tests/context_groups.rs:473` — `sleep(20ms)` pour changer le mtime : flaky selon le FS.
- **T6 [M]** Benches/exemples dupliqués : `LUA_SOURCE/JS_SOURCE/TS_SOURCE` et impls `Inc`/`ScoreUpdate` identiques entre `benches/vs_lua.rs` et `examples/native_roundtrip.rs` ; `benches/native_bridge.rs` (921 l, ~40 impls `HostContract` répétitives → `macro_rules!`) ; `benches/runtime.rs:333-552` mélange mesure mémoire `/proc` et Criterion.
- **T7 [L]** `src/compiler/resolver.rs:391 temp_fixture_dir` ne nettoie jamais (utiliser `tempfile::TempDir`, déjà en dépendance). `manifest.rs:585` assertion faible (ligne 3 ou 4). `cache/identity.rs` : aucun test de changement de clé selon `CACHE_SCHEMA_VERSION`. `packages.rs`/`package_permissions.rs`/`script_export.rs` : `.unwrap()` au lieu de `.expect("create engine")`.
- **T8 [L]** `tests/cli_init.rs:61` : un sous-programme (`export_child`) piloté par `MANIVELLE_TEST_HANG` dans un `#[test]` → exemple/binaire dédié.
- **C1 [M]** `Cargo.toml:12` — `exclude` ne liste pas `tests/`, `benches/`, `examples/`, `docs/`, `assets/` (vérifier avec `cargo package --list`). `tests/projects/**/node_modules` est **tracké dans git** (11 fichiers) : voulu pour les fixtures, mais à confirmer + à exclure du package publié.
- **C2 [L]** `.gitignore:8` contient `AGENTS.md` ; `docs/` : tous les liens `.md` et APIs citées existent (vérifié, rien à corriger).

---

## 10. Bruit clippy non retenu (pedantic/nursery)

À NE PAS faire en masse (peu de valeur ou arbitraire) : 98 `doc_markdown` (backticks), 94 `float_cmp` (surtout tests), 64 `missing_errors_doc`, ~150 `redundant_pub_crate`, 41 `use_self`, 41 `missing_const_for_fn`, 32 `needless_raw_string_hashes`, 17 `format_push_string`. Quelques-uns utiles : 18 `needless_pass_by_value` (dont `errors.rs:14,38`, `lifecycle.rs:704,723`, `host_resolver.rs:173`, `bindings.rs:319`), 1 `redundant_clone` (`analysis/check.rs:289`), 1 `match_same_arms` (`declarations.rs:345`), 1 `unnecessary_wraps` (`module_loader/store/insertion.rs:93`), 7 casts `usize→u32` tronquants, 3 casts `*mut u8 → *mut u64` (alignement, `quota.rs`).

---

## 11. Ordre de travail suggéré

1. **Bugs** : B1, B2, B3, B4, B5, B6, B7, B8 (petits, isolés, testables).
2. **Dédup du générateur TS** : D1 + D2 + B1/B2 ensemble (un seul modèle de contrat → deux rendus, une liste de mots réservés).
3. **Décision d'API** : N2 (supprimer le trait `HostContractRegistry` ou l'API inhérente) — casse semver, à décider avant 0.2.
4. **Découpe** : G1/G2 (`lifecycle.rs`, `Engine`), puis G3/G4/G5/G6.
5. **Perf** : P1 (bytes), P2 (chemins paresseux), P3/P4/P5.
6. **Typage** : I1/I2/I3 (enums à la place des chaînes/sentinelles).
7. **Tests** : T1–T4 (helpers partagés d'abord, c'est mécanique), puis T5–T8.
8. **Polish** : le reste des [L].

Dis-moi les IDs à faire (ex. « B1–B8, D1, D2, G1 ») et ceux à ignorer.

---

## 12. Suivi des corrections (mis à jour après le passage de fix)

Chaque point a été revérifié avant correction. Légende : **FAIT**, **PARTIEL**, **FAUX POSITIF** (le code est correct, rien changé), **NON FAIT** (raison donnée). Vérification finale : `cargo clippy --all-targets --all-features` à 0 warning, `cargo doc --no-deps --all-features` à 0 warning, `cargo test --all-features --no-fail-fast` : 58 suites, 517 tests passés, 0 échec.

### Bugs (§1)
| ID | Statut | Détail |
|---|---|---|
| B1 | FAIT | Une seule liste de mots réservés (`host_contracts/names.rs`). |
| B2 | FAIT | Noms invalides → suffixe unique (`sdk`, `sdk2`…) au lieu d'un doublon `export const sdk`. |
| B3 | FAIT | `LineIndex` partagé (octets, plus de calcul quadratique) + test de régression non-ASCII. |
| B4 | FAIT | `assert!` sur `MAX_OWNERS` avant la conversion. |
| B5 | FAIT | Doublon de contrat → `EngineError::DuplicateContract`. `script_export` garde volontairement l'écrasement (un test l'enregistre deux fois). |
| B6 | FAUX POSITIF | `UserData::insert` rend la valeur en cas d'erreur et `FieldAtoms::drop` libère l'atome. |
| B7 | PARTIEL | `get_index` ne peut pas échouer aujourd'hui ; ajout d'un `continue` défensif seulement. |
| B8 | FAIT | `take()` rapporte toutes les rejections ; test ajouté. |
| B9 | FAIT | `validate_union` détaille l'erreur de chaque membre ; test ajouté. |
| B10 | FAUX POSITIF | Chemin vide = « atteint via le SDK seulement », voulu et commenté ; l'entrée de module est nécessaire à `import_module_names()`. |
| B11 | FAUX POSITIF | `JS_NewAtomLen` prend un `size_t` (u64) : `len() as u64` ne tronque pas. Un correctif `i32::try_from` a été tenté puis annulé. |
| B12 | FAIT | `getrandom::fill` (32 octets, hex). Dépendance `getrandom` ajoutée. |
| B13 | FAIT | `GENERATED_SDK_MARKER` émis en tête du SDK généré et lu par `is_generated`. Les SDK générés par une ancienne version (sans marqueur) ne sont plus reconnus : il faut les régénérer. |

### Duplication (§2)
| ID | Statut | Détail |
|---|---|---|
| D1 | FAIT | Trait `DescriptorSink` + `push_descriptor` partagé. |
| D2 | FAIT | `ObjectTree<T>` générique + `names.rs`. |
| D3 | FAIT | `described()` partagé. `script_export` garde l'écrasement. |
| D4 | PARTIEL | `call_validated` partagé. `ValidatedReply::into_js` garde son chemin : il lève via `Exception::throw_message`, pas `js_host_error` (comportement différent). |
| D5, D6, D7, D8 | FAIT | `run_in_script`, `near_limit`/`run_jobs`, `decode_reply`, helper de prototype partagé. |
| D9, D11, D12 | FAIT | |
| D10 | FAIT | `canonical_roots` dans `src/bin/manivelle/roots.rs` (pas dans `packages`). |
| D13 | PARTIEL | `call_discarding` factorisé ; `NextTimer`/`EarliestDue` gardés séparés (sémantiques différentes). |
| D14 | FAIT | `resolver.rs`/`parent_dir` : faux positif (fallback silencieux sans intérêt). |
| D15 | FAUX POSITIF | Les deux chemins appellent déjà le même writer ; fonctions publiques. |

### Trop long / trop gros (§3)
| ID | Statut | Détail |
|---|---|---|
| G1 | FAIT | `lifecycle.rs` → `load`, `hot_reload`, `mount`, `prelude`, `context`. |
| G2 | PARTIEL | `Engine` regroupé en `Scheduling`/`Faults`/`Shared`. `EngineScript` non regroupé (un seul constructeur : simple indirection). |
| G3, G4, G6, G7, G8 | FAIT | `sdk/` éclaté ; bin en `src/bin/manivelle/{main,cli…}.rs` (`fn run` → `dispatch`) ; `attrs/` éclaté ; `errors.rs`→`source_locations.rs`. |
| G5 | PARTIEL | Découverte, noms et édition KDL extraites de `packages/mod.rs`. `manifest.rs` (couche `Node`/`Failure`) gardé : couche volontaire avec lignes de span. |
| Fonctions longues | FAIT | `manifest`, `check_usage`, `discover_project`, `signature`, `connect`, `setup_protocol`, `mount`, `run_target`, `add_permissions`, `deliver`. |

### Non-idiomatique / noms (§4, §5)
| ID | Statut | Détail |
|---|---|---|
| I1 | PARTIEL | `Direction` enum fait. `ScriptId` newtype et champs `String` d'`EngineError` : NON FAIT (changement d'API large, bénéfice faible). |
| I2 | PARTIEL | `Deadline` enum, `bridge.rs` sans sentinelles. Sentinelle `"unknown"` des schémas : NON FAIT (nombreux appelants publics). |
| I3 | NON FAIT | `rquickjs::Error` ne porte qu'un message ; un chemin structuré change la signature `#[doc(hidden)] pub` `at_path`. |
| I4 | NON FAIT | `js_host_error` : conversion imposée par `rquickjs::Error` côté hôte, à reprendre avec I3. |
| I5, I7, I9 | FAIT | |
| I6 | PARTIEL | `'a` → `'_` : faux positif (`new` utilise `'a`). Le reste fait. |
| I8 | PARTIEL | `resolve.rs` itératif ; `compiler/project.rs::visit` reste récursif (profondeur bornée par les imports du projet). |
| I10 | PARTIEL | Invariants documentés ou `expect` retirés ; `retained.rs:55` gardé (2^31 arguments impossibles). |
| I11 | FAUX POSITIF/NON FAIT | `check.rs` ne renvoie pas de `String` ; `validate_json*` est public. |
| N1 | PARTIEL | `descriptor(name)` → `get`. `MapKey`/`TsRecordKey` : documentés (pas de renommage d'API publique). |
| N2 | FAIT | Trait `HostContractRegistry` supprimé ; API chaînée inhérente (`function`, `callback`, …, `get`). Voir CHANGELOG (breaking). |
| N3 | NON FAIT | Renommages d'API publique (`HostRequest`…) non décidés. |
| N4 | PARTIEL | `Engine::dispatch` → `run_audience`. Autres noms de `bridge.rs`/`codec.rs` inchangés. |
| N5 | FAIT | `_ty`, `rust_name`, `context_run_end`→`run_len`. |
| N6 | NON FAIT | `__derive` doit rester public (les macros génèrent `::manivelle::__derive::…`). |
| N7 | FAIT | |
| N8 | FAUX POSITIF (partiel) | `PendingCall::is_finished/take` sont de l'API publique. |

### Modules (§6)
| ID | Statut | Détail |
|---|---|---|
| M1 | PARTIEL | État runtime déplacé dans `runner/reply_inbox.rs` ; `HostResolver` public reste dans `contract/`. |
| M2, M3, M5, M6 | FAIT | |

### Perf (§7)
| ID | Statut | Détail |
|---|---|---|
| P1, P2, P4 | FAIT | Snapshot binaire sans `Value` par octet ; chemins paresseux ; `HashMap`/`LineIndex`. |
| P3 | FAUX POSITIF | Les scans ne coûtent que s'il y a du travail ; clone sur chemin d'erreur. |
| P5 | NON FAIT | Le clone vient du type `DiscoveredModule.source: String` consommé par le runner ; `Arc` testé sans gain. |
| P6 | PARTIEL | `snapshot()` O(1) (Arc/COW). `list()`/`descriptors()` clonent toujours (appelés au setup seulement). |
| P7 | FAUX POSITIF | Le détour texte garde la forme décimale la plus courte (`1.1f32` → `1.1`) : voulu. |
| P8 | NON FAIT | Chemins rares ou petits ; `active_tasks` gardé (fast path O(1)). `import_modules` en `&mut String` : fait. |
| P9 | PARTIEL | `edit_distance`, `one_arg` faits. `cache/store.rs` : copies nécessaires. |

### unsafe (§8)
S1, S2, S3, S4 : FAIT.

### Tests / benches / config (§9)
| ID | Statut | Détail |
|---|---|---|
| T1 | FAIT | `derive_schema.rs` → `derive_schema_{shapes,flatten,enums,host}.rs` (33 tests, mêmes assertions). |
| T2 | PARTIEL | `engine`, `engine_projects`, `context_groups` éclatés (82 tests inchangés). `type_contract_derive.rs` (harnais et types communs) et tests inline de `runner/` : NON FAIT. |
| T3 | PARTIEL | `support` : `assert_tsc`, `run_tsc*`, `write_file`, `stdout/stderr`, `count`, `write_tsconfig`, `wait_until`…. Les `engine()` avec options ou setup propre restent locaux. |
| T4 | PARTIEL | `host_registry_api.rs`, `dogfood_usage.rs` : assertions de texte remplacées par des contrôles `tsc`. `schema.rs` non revu. |
| T5 | FAUX POSITIF | `FileStamp` compare taille + mtime, détection déterministe ; `sleep` retiré par hygiène. |
| T6 | PARTIEL | `Inc`/`ScoreUpdate` partagés (`benches/support/workload.rs`), macro dans `native_bridge.rs` (8 contrats, pas ~40), mesure mémoire sortie de `runtime.rs`. Les sources LUA/JS/TS divergent : pas de doublon réel. |
| T7 | FAIT | |
| T8 | NON FAIT | `export_child` doit rester dans le binaire de test (`CARGO_BIN_EXE_*` indisponible pour un exemple). |
| C1 | FAIT | `exclude` : `tests`, `benches`, `docs` ajoutés (`cargo package --list` vérifié). Cargo avertit que les cibles `[[test]]`/`[[bench]]` sont ignorées dans le paquet publié : cosmétique. |
| C2 | FAUX POSITIF | `AGENTS.md` dans `.gitignore` est voulu (outillage local). |
