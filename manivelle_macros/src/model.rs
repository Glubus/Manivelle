//! Resolved view of a derive input: serde names, shapes and tagging, computed once so the
//! schema and the codec read the same facts.

use syn::{
    Data, DeriveInput, Error, Fields, GenericArgument, Generics, Ident, Member, PathArguments,
    Result, Type,
};

use crate::attrs::{ContainerAttrs, DefaultValue, FieldAttrs, VariantAttrs};
use crate::directions::Directions;
use crate::rename::{Name, RenameRules};

/// The deriving type.
pub(crate) struct Container<'a> {
    pub(crate) ident: &'a Ident,
    pub(crate) generics: &'a Generics,
    pub(crate) attrs: ContainerAttrs,
    pub(crate) body: Body<'a>,
}

pub(crate) enum Body<'a> {
    Struct(Shape<'a>),
    Enum(Vec<Variant<'a>>),
}

/// Field layout of a struct or enum variant, in serde's terms.
pub(crate) struct Shape<'a> {
    pub(crate) style: Style,
    pub(crate) fields: Vec<Field<'a>>,
    pub(crate) original: &'a Fields,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// No fields: `null` for structs, a bare name for variants.
    Unit,
    /// One unnamed field: the inner value.
    Newtype,
    /// Zero or several unnamed fields: an array.
    Tuple,
    /// Named fields: an object.
    Struct,
}

pub(crate) struct Field<'a> {
    pub(crate) member: Member,
    pub(crate) ty: &'a Type,
    pub(crate) name: Name,
    pub(crate) attrs: FieldAttrs,
    pub(crate) original: &'a syn::Field,
}

pub(crate) struct Variant<'a> {
    pub(crate) ident: &'a Ident,
    pub(crate) name: Name,
    pub(crate) shape: Shape<'a>,
    pub(crate) attrs: VariantAttrs,
    pub(crate) original: &'a syn::Variant,
}

/// Serde enum representation.
pub(crate) enum Tagging<'a> {
    External,
    Internal { tag: &'a str },
    Adjacent { tag: &'a str, content: &'a str },
    Untagged,
}

/// Source of a field value that the decoded input does not provide.
pub(crate) enum Missing<'f> {
    /// `#[serde(default)]` or `#[serde(default = "path")]` on the field.
    Default(&'f DefaultValue),
    /// The member of the struct's own `#[serde(default)]` value.
    ContainerDefault,
    /// No default: missing is an error, or `None` for `Option` fields.
    Required,
}

impl<'a> Container<'a> {
    pub(crate) fn from_input(input: &'a DeriveInput) -> Result<Self> {
        let attrs = ContainerAttrs::from_attrs(&input.attrs)?;
        let body = match &input.data {
            Data::Struct(data) => Body::Struct(Shape::from_fields(&data.fields, attrs.rename_all)?),
            Data::Enum(data) => Body::Enum(
                data.variants
                    .iter()
                    .map(|variant| Variant::from_variant(variant, &attrs))
                    .collect::<Result<_>>()?,
            ),
            Data::Union(_) => {
                return Err(Error::new_spanned(
                    input,
                    "TsSchema cannot be derived for unions",
                ));
            }
        };
        Ok(Self {
            ident: &input.ident,
            generics: &input.generics,
            attrs,
            body,
        })
    }

    /// Rust type name used in conversion errors.
    pub(crate) fn rust_name(&self) -> String {
        self.ident.to_string()
    }

    pub(crate) fn tagging(&self) -> Tagging<'_> {
        match (&self.attrs.tag, &self.attrs.content) {
            _ if self.attrs.untagged => Tagging::Untagged,
            (Some(tag), Some(content)) => Tagging::Adjacent { tag, content },
            (Some(tag), None) => Tagging::Internal { tag },
            (None, _) => Tagging::External,
        }
    }

    /// Codecs emitted for the type.
    pub(crate) fn codecs(&self) -> Directions {
        self.attrs.codecs
    }

    /// Directions the single TypeScript schema describes: the emitted codecs, or both
    /// for schema-only types.
    pub(crate) fn schema_directions(&self) -> Directions {
        if self.codecs() == Directions::NONE {
            Directions::BOTH
        } else {
            self.codecs()
        }
    }

    /// Key the schema shows for `name`: the input key for decode-only types, else the
    /// output key.
    pub(crate) fn schema_key<'n>(&self, name: &'n Name) -> &'n str {
        if self.schema_directions() == Directions::DECODE {
            &name.deserialize
        } else {
            &name.serialize
        }
    }

    /// Type serde really reads or writes for `#[serde(into/from/try_from)]` containers.
    pub(crate) fn wire_type(&self) -> Option<&Type> {
        let prefers_into = self.schema_directions().encode;
        self.attrs
            .into
            .as_ref()
            .filter(|_| prefers_into)
            .or(self.attrs.from.as_ref())
            .or(self.attrs.into.as_ref())
    }

    /// The field a `#[serde(transparent)]` struct forwards to.
    pub(crate) fn transparent_field(&self) -> Option<&Field<'a>> {
        match &self.body {
            Body::Struct(shape) if self.attrs.transparent => shape.transparent_field(),
            _ => None,
        }
    }
}

impl<'a> Shape<'a> {
    fn from_fields(fields: &'a Fields, rules: RenameRules) -> Result<Self> {
        let style = match fields {
            Fields::Unit => Style::Unit,
            Fields::Named(_) => Style::Struct,
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => Style::Newtype,
            Fields::Unnamed(_) => Style::Tuple,
        };
        let fields_resolved = fields
            .iter()
            .enumerate()
            .map(|(index, field)| Field::from_field(field, index, rules))
            .collect::<Result<_>>()?;
        Ok(Self {
            style,
            fields: fields_resolved,
            original: fields,
        })
    }

    pub(crate) fn has_flatten(&self) -> bool {
        self.fields.iter().any(|field| field.attrs.flatten)
    }

    /// Serde's transparent field: the only field, or the only one that is not
    /// `PhantomData`, skipped or defaulted.
    fn transparent_field(&self) -> Option<&Field<'a>> {
        if let [field] = self.fields.as_slice() {
            return Some(field);
        }
        let mut candidates = self.fields.iter().filter(|field| field.is_forwarded());
        match (candidates.next(), candidates.next()) {
            (Some(field), None) => Some(field),
            _ => None,
        }
    }
}

impl<'a> Field<'a> {
    fn from_field(field: &'a syn::Field, index: usize, rules: RenameRules) -> Result<Self> {
        let attrs = FieldAttrs::from_field(field)?;
        let (member, name) = match &field.ident {
            Some(ident) => (
                Member::Named(ident.clone()),
                Name::field(ident, &attrs.rename, rules, &attrs.aliases),
            ),
            None => (Member::Unnamed(index.into()), Name::index(index)),
        };
        Ok(Self {
            member,
            ty: &field.ty,
            name,
            attrs,
            original: field,
        })
    }

    /// Inner type of a syntactic `Option<T>` field.
    pub(crate) fn option_inner(&self) -> Option<&'a Type> {
        let Type::Path(path) = self.ty else {
            return None;
        };
        let segment = path.path.segments.last().filter(|_| path.qself.is_none())?;
        let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        match arguments.args.first() {
            Some(GenericArgument::Type(inner)) if segment.ident == "Option" => Some(inner),
            _ => None,
        }
    }

    /// Where the decoded value comes from when the key is missing. `container_default`
    /// is true inside a struct carrying `#[serde(default)]` itself.
    pub(crate) fn missing(&self, container_default: bool) -> Missing<'_> {
        match &self.attrs.default {
            Some(default) => Missing::Default(default),
            None if container_default => Missing::ContainerDefault,
            None => Missing::Required,
        }
    }

    fn is_phantom(&self) -> bool {
        let Type::Path(path) = self.ty else {
            return false;
        };
        path.path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "PhantomData")
    }

    fn is_forwarded(&self) -> bool {
        !self.is_phantom()
            && !self.attrs.skip_serializing
            && !self.attrs.skip_deserializing
            && self.attrs.default.is_none()
    }
}

impl<'a> Variant<'a> {
    fn from_variant(variant: &'a syn::Variant, container: &ContainerAttrs) -> Result<Self> {
        let attrs = VariantAttrs::from_variant(variant)?;
        let field_rules = attrs.rename_all.or(container.rename_all_fields);
        Ok(Self {
            ident: &variant.ident,
            name: Name::variant(
                &variant.ident,
                &attrs.rename,
                container.rename_all,
                &attrs.aliases,
            ),
            shape: Shape::from_fields(&variant.fields, field_rules)?,
            attrs,
            original: variant,
        })
    }

    /// Whether the variant is left out of the given directions.
    pub(crate) fn skipped_in(&self, directions: Directions) -> bool {
        (!directions.encode || self.attrs.skip_serializing)
            && (!directions.decode || self.attrs.skip_deserializing)
    }
}
