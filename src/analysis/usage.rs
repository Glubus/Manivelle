//! Which host contracts the TypeScript of a package uses, found by reading its code.

use std::collections::HashMap;

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Argument, CallExpression, ComputedMemberExpression, Declaration, ExportNamedDeclaration,
    Expression, IdentifierReference, ImportDeclaration, ImportDeclarationSpecifier,
    ModuleExportName, Program, Statement, StaticMemberExpression,
};
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::semantic::{Scoping, SemanticBuilder};
use oxc::span::SourceType;

use super::{ContractNames, ScriptUsage, Usage};

/// The import specifier of the environment module: `ctx`, `console` and the timers.
const ENV_MODULE: &str = "manivelle:env";

/// Reads one module and adds what it uses to `usage`. `file` names it in the findings;
/// `is_entry` also collects the names it exports. A source that does not parse is left
/// to the transpiler to report.
pub(super) fn analyze_module(
    source: &str,
    file: &str,
    is_entry: bool,
    contracts: &ContractNames,
    usage: &mut ScriptUsage,
) {
    let source_type = SourceType::from_path(file)
        .map(|source_type| source_type.with_module(true))
        .unwrap_or_else(|_| SourceType::ts().with_module(true));
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return;
    }
    let program = parsed.program;
    let semantic = SemanticBuilder::new().build(&program).semantic;

    let mut reader = Reader {
        source,
        file,
        scoping: semantic.scoping(),
        contracts,
        imports: imports_of(&program),
        usage,
    };
    reader.record_host_imports(&program);
    reader.visit_program(&program);
    if is_entry {
        reader.usage.collect_exports(&program);
    }
}

/// How a name the module imported is read.
#[derive(Clone)]
struct Import {
    /// The module it comes from.
    source: String,
    /// The name it has there (`ctx` for `import { ctx as c }`).
    imported: String,
    /// `import * as sdk`: the binding is the whole module.
    namespace: bool,
}

fn imports_of(program: &Program<'_>) -> HashMap<String, Import> {
    let mut imports = HashMap::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        let source = declaration.source.value.to_string();
        for specifier in declaration.specifiers.iter().flatten() {
            let (local, imported, namespace) = match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(named) => (
                    named.local.name.to_string(),
                    export_name(&named.imported),
                    false,
                ),
                ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                    (default.local.name.to_string(), "default".to_owned(), false)
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(all) => {
                    (all.local.name.to_string(), "*".to_owned(), true)
                }
            };
            imports.insert(
                local,
                Import {
                    source: source.clone(),
                    imported,
                    namespace,
                },
            );
        }
    }
    imports
}

fn export_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(name) => name.name.to_string(),
        ModuleExportName::IdentifierReference(name) => name.name.to_string(),
        ModuleExportName::StringLiteral(name) => name.value.to_string(),
    }
}

/// What a chain of property accesses starts from.
#[derive(PartialEq, Eq)]
enum Root {
    /// A global: how a host namespace reaches a script.
    Global,
    /// A binding of `manivelle:env` or of the generated SDK, read like a global.
    Imported,
    /// Anything else: a local, a parameter, an import of the script's own.
    Other,
}

struct Reader<'s> {
    source: &'s str,
    file: &'s str,
    scoping: &'s Scoping,
    contracts: &'s ContractNames,
    imports: HashMap<String, Import>,
    usage: &'s mut ScriptUsage,
}

impl Reader<'_> {
    fn usage_at(&self, name: String, offset: u32) -> Usage {
        let line = 1 + self.source[..offset as usize].matches('\n').count();
        Usage {
            name,
            file: self.file.to_owned(),
            line,
        }
    }

    /// `import { highScore } from "game"`: a host function reached through a host module.
    fn record_host_imports(&mut self, program: &Program<'_>) {
        for statement in &program.body {
            let Statement::ImportDeclaration(declaration) = statement else {
                continue;
            };
            self.record_host_import(declaration);
        }
    }

    fn record_host_import(&mut self, declaration: &ImportDeclaration<'_>) {
        let source = declaration.source.value.as_str();
        if !self.contracts.host_modules.contains(source) {
            return;
        }
        for specifier in declaration.specifiers.iter().flatten() {
            let ImportDeclarationSpecifier::ImportSpecifier(named) = specifier else {
                continue;
            };
            let imported = export_name(&named.imported);
            let key = (source.to_owned(), imported);
            if let Some(contract) = self.contracts.by_import.get(&key) {
                let found = self.usage_at(contract.clone(), named.span.start);
                self.record_contract(found);
            }
        }
    }

    /// Records a contract the code reaches, as a function or as an event.
    fn record_contract(&mut self, found: Usage) {
        if self.contracts.events.contains(&found.name)
            && !self.contracts.functions.contains(&found.name)
        {
            self.usage.events.push(found);
        } else {
            self.usage.functions.push(found);
        }
    }

    fn root_kind(&self, root: &IdentifierReference<'_>) -> Root {
        let symbol = root
            .reference_id
            .get()
            .and_then(|reference| self.scoping.get_reference(reference).symbol_id());
        match (symbol, self.imports.get(root.name.as_str())) {
            (None, _) => Root::Global,
            (Some(_), Some(import)) if self.reads_like_a_global(import) => Root::Imported,
            _ => Root::Other,
        }
    }

    fn reads_like_a_global(&self, import: &Import) -> bool {
        import.source == ENV_MODULE
            || import.source.ends_with(".sdk")
            || import.source.ends_with(".sdk.ts")
            || import.source.ends_with(".sdk.js")
            || self.contracts.host_modules.contains(&import.source)
    }

    /// The dotted path of a chain `root.a.b`, from the name the chain's root has for the
    /// host, and how the root is read; `None` when it is not such a chain.
    fn chain(&self, expression: &Expression<'_>) -> Option<(Vec<String>, Root)> {
        let mut segments = Vec::new();
        let mut current = expression;
        loop {
            match current {
                Expression::StaticMemberExpression(member) => {
                    segments.push(member.property.name.to_string());
                    current = &member.object;
                }
                Expression::Identifier(root) => {
                    segments.reverse();
                    let kind = self.root_kind(root);
                    let mut path = Vec::with_capacity(segments.len() + 1);
                    match self.imports.get(root.name.as_str()) {
                        Some(import) if kind == Root::Imported => {
                            if !import.namespace {
                                path.push(import.imported.clone());
                            }
                        }
                        _ => path.push(root.name.to_string()),
                    }
                    path.extend(segments);
                    return Some((path, kind));
                }
                _ => return None,
            }
        }
    }

    fn reach(&mut self, path: &[String], kind: &Root, offset: u32) {
        if *kind == Root::Other || path.is_empty() {
            return;
        }
        // `events.user.found(handler)` of the SDK listens to `user.found`.
        if kind == &Root::Imported && path[0] == "events" && path.len() > 1 {
            let found = self.usage_at(path[1..].join("."), offset);
            self.usage.events.push(found);
            return;
        }
        let longest = (1..=path.len())
            .rev()
            .map(|length| path[..length].join("."))
            .find(|name| self.contracts.functions.contains(name));
        if let Some(name) = longest {
            let found = self.usage_at(name, offset);
            self.usage.functions.push(found);
        } else if *kind == Root::Global
            && path.len() > 1
            && self.contracts.namespaces.contains(&path[0])
        {
            // Under the name of a host namespace, but not a function of it: a typo. The
            // SDK's own event aliases (`user.onFound`) are not a global's.
            let found = self.usage_at(path.join("."), offset);
            self.usage.unknown.push(found);
        }
    }
}

impl<'a> Visit<'a> for Reader<'_> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        // A host function with no namespace, reached by its bare name.
        if self.contracts.functions.contains(it.name.as_str()) && self.root_kind(it) == Root::Global
        {
            let found = self.usage_at(it.name.to_string(), it.span.start);
            self.usage.functions.push(found);
        }
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if let Some((path, kind)) = self.chain_of_member(it)
            && kind != Root::Other
        {
            self.reach(&path, &kind, it.span.start);
            return;
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        if let Some((path, kind)) = self.chain(&it.object)
            && kind != Root::Other
            && self.contracts.is_namespace_path(&path)
        {
            let found = self.usage_at(format!("{}[...]", path.join(".")), it.span.start);
            self.usage.dynamic.push(found);
        }
        walk::walk_computed_member_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Some((path, kind)) = self.chain(&it.callee)
            && kind != Root::Other
            && path.len() == 2
            && path[0] == "ctx"
            && matches!(path[1].as_str(), "on" | "off")
        {
            match it.arguments.first() {
                Some(Argument::StringLiteral(event)) => {
                    let found = self.usage_at(event.value.to_string(), event.span.start);
                    self.usage.events.push(found);
                }
                _ => {
                    let found = self.usage_at(format!("ctx.{}(...)", path[1]), it.span.start);
                    self.usage.dynamic.push(found);
                }
            }
        }
        walk::walk_call_expression(self, it);
    }
}

impl Reader<'_> {
    /// The chain a member expression ends, as [`Reader::chain`] reads it.
    fn chain_of_member(&self, member: &StaticMemberExpression<'_>) -> Option<(Vec<String>, Root)> {
        let (mut path, kind) = self.chain(&member.object)?;
        path.push(member.property.name.to_string());
        Some((path, kind))
    }
}

impl ScriptUsage {
    /// The names a module exports: `export function f`, `export const f`, `export { a as
    /// b }` and `export default`.
    fn collect_exports(&mut self, program: &Program<'_>) {
        for statement in &program.body {
            match statement {
                Statement::ExportNamedDeclaration(export) => self.collect_named_export(export),
                Statement::ExportDefaultDeclaration(_) => {
                    self.exports.insert("default".to_owned());
                }
                Statement::ExportAllDeclaration(_) => self.re_exports_all = true,
                _ => {}
            }
        }
    }

    fn collect_named_export(&mut self, export: &ExportNamedDeclaration<'_>) {
        if export.export_kind.is_type() {
            return;
        }
        match &export.declaration {
            Some(Declaration::FunctionDeclaration(function)) => {
                if let Some(id) = &function.id {
                    self.exports.insert(id.name.to_string());
                }
            }
            Some(Declaration::ClassDeclaration(class)) => {
                if let Some(id) = &class.id {
                    self.exports.insert(id.name.to_string());
                }
            }
            Some(Declaration::VariableDeclaration(variables)) => {
                for declarator in &variables.declarations {
                    for name in declarator.id.get_binding_identifiers() {
                        self.exports.insert(name.name.to_string());
                    }
                }
            }
            _ => {}
        }
        for specifier in &export.specifiers {
            if specifier.export_kind.is_value() {
                self.exports.insert(export_name(&specifier.exported));
            }
        }
        if export.source.is_some() && export.specifiers.is_empty() {
            self.re_exports_all = true;
        }
    }
}

/// The module paths and sources of the files a project is made of that the analysis
/// reads: the generated SDK defines the calls it forwards, so it is not the package's.
pub(super) fn is_generated(source: &str, file: &str) -> bool {
    file.ends_with(".d.ts") || source.contains("function __hostCall")
}
