/*
 * This file is part of Astarte.
 *
 * Copyright 2023-2024 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

//! Proc macro helpers for the [Astarte Device SDK](https://crates.io/crates/astarte-device-sdk)

use std::{collections::HashMap, fmt::Debug};

use proc_macro::TokenStream;

use proc_macro2::Ident;
use quote::{quote, quote_spanned};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    Attribute, Expr, GenericParam, Generics, MetaNameValue, Token,
};

use crate::{case::RenameRule, event::FromEventDerive};

mod case;
mod event;

/// Handle for the `#[astarte_aggregate(..)]` attribute.
///
/// ### Example
///
/// ```no_compile
/// #[derive(AstarteAggregate)]
/// #[astarte_aggregate(rename_all = "camelCase")]
/// struct Foo {
///     bar_v: String
/// }
/// ```
#[derive(Debug, Default)]
struct AggregateAttributes {
    /// Rename the fields in the resulting HashMap, see the [`RenameRule`] variants.
    rename_all: Option<RenameRule>,
}

impl AggregateAttributes {
    /// Merge the Astarte attributes from the other struct into self.
    fn merge(self, other: Self) -> Self {
        let rename_all = other.rename_all.or(self.rename_all);

        Self { rename_all }
    }
}

impl Parse for AggregateAttributes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut vars = parse_name_value_attrs(input)?;

        let rename_all = vars
            .remove("rename_all")
            .map(|expr| {
                parse_str_lit(&expr).and_then(|rename| {
                    RenameRule::from_str(&rename)
                        .map_err(|_| syn::Error::new(expr.span(), "invalid rename rule"))
                })
            })
            .transpose()?;

        Ok(Self { rename_all })
    }
}

/// Parses the content of a [`syn::MetaList`] as a list of [`syn::MetaNameValue`].
///
/// Will convert a list of `#[attr(name = "string",..)]` into an [`HashMap<String, string>`]
fn parse_name_value_attrs(
    input: &syn::parse::ParseBuffer<'_>,
) -> Result<HashMap<String, Expr>, syn::Error> {
    Punctuated::<MetaNameValue, Token![,]>::parse_terminated(input)?
        .into_iter()
        .map(|v| {
            v.path
                .get_ident()
                .ok_or_else(|| {
                    syn::Error::new(v.span(), "expected an identifier like `rename_all`")
                })
                .map(|i| (i.to_string(), v.value))
        })
        .collect::<syn::Result<_>>()
}

/// Parses a [`syn::Lit::Str`] into a [`String`].
fn parse_str_lit(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(lit),
            ..
        }) => Ok(lit.value()),
        _ => Err(syn::Error::new(
            expr.span(),
            "expression must be a string literal",
        )),
    }
}

/// Parses a [`syn::Lit::Bool`] into a [`bool`].
fn parse_bool_lit(expr: &Expr) -> syn::Result<bool> {
    match expr {
        Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Bool(lit),
            ..
        }) => Ok(lit.value()),
        _ => Err(syn::Error::new(
            expr.span(),
            "expression must be a bool literal",
        )),
    }
}

/// Handle for the `#[derive(AstarteAggregate)]` derive macro.
///
/// ### Example
///
/// ```no_compile
/// #[derive(AstarteAggregate)]
/// struct Foo {
///     bar: String
/// }
/// ```
struct AggregateDerive {
    name: Ident,
    attrs: AggregateAttributes,
    fields: Vec<Ident>,
    generics: Generics,
}

impl AggregateDerive {
    fn quote(&self) -> proc_macro2::TokenStream {
        let rename_rule = self.attrs.rename_all.unwrap_or_default();

        let name = &self.name;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();
        let fields = self.fields.iter().map(|i| {
            let name = i.to_string();
            let name = rename_rule.apply_to_field(&name);
            quote_spanned! {i.span() =>
                // TODO *Temporarily* ignore this new lint will be fixed in a new pr
                #[allow(unknown_lints)]
                #[allow(clippy::unnecessary_fallible_conversions)]
                let value: astarte_device_sdk::types::AstarteType = std::convert::TryInto::try_into(self.#i)?;
                result.insert(#name.to_string(), value);
            }
        });

        quote! {
            impl #impl_generics astarte_device_sdk::AstarteAggregate for #name #ty_generics #where_clause {
                fn astarte_aggregate(
                    self,
                ) -> Result<
                    std::collections::HashMap<String, astarte_device_sdk::types::AstarteType>,
                    astarte_device_sdk::error::Error,
                > {
                    let mut result = std::collections::HashMap::new();
                    #(#fields)*
                    Ok(result)
                }
            }
        }
    }

    pub fn add_trait_bound(mut generics: Generics) -> Generics {
        for param in &mut generics.params {
            if let GenericParam::Type(ref mut type_param) = *param {
                type_param.bounds.push(parse_quote!(
                    std::convert::TryInto<astarte_device_sdk::types::AstarteType, Error = astarte_device_sdk::error::Error>
                ));
            }
        }
        generics
    }
}

impl Parse for AggregateDerive {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ast = syn::DeriveInput::parse(input)?;

        // Find all the outer astarte_aggregate attributes and merge them
        let attrs = ast
            .attrs
            .iter()
            .filter_map(|a| parse_attribute_list::<AggregateAttributes>(a, "astarte_aggregate"))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .reduce(|first, second| first.merge(second))
            .unwrap_or_default();

        let fields = parse_struct_fields(&ast)?;

        let name = ast.ident;

        let generics = Self::add_trait_bound(ast.generics);

        Ok(Self {
            name,
            attrs,
            fields,
            generics,
        })
    }
}

/// Parses the fields of a struct
fn parse_struct_fields(ast: &syn::DeriveInput) -> Result<Vec<Ident>, syn::Error> {
    let syn::Data::Struct(ref st) = ast.data else {
        return Err(syn::Error::new(ast.span(), "a named struct is required"));
    };
    let syn::Fields::Named(ref fields_named) = st.fields else {
        return Err(syn::Error::new(ast.span(), "a nemed struct is required"));
    };

    let fields = fields_named
        .named
        .iter()
        .map(|field| {
            field
                .ident
                .clone()
                .ok_or_else(|| syn::Error::new(field.span(), "field is not an ident"))
        })
        .collect::<Result<_, _>>()?;

    Ok(fields)
}

/// Parse the `#[name(..)]` attribute.
///
/// This will skip other attributes or return an error if the attribute parsing failed. We expected
/// the input to be an outer attribute in the form `#[name(foo = "...")]`.
pub(crate) fn parse_attribute_list<T>(attr: &Attribute, name: &str) -> Option<syn::Result<T>>
where
    T: Parse,
{
    let is_attr = attr
        .path()
        .get_ident()
        .map(ToString::to_string)
        .filter(|ident| ident == name)
        .is_some();

    if !is_attr {
        return None;
    }

    // TODO: outer and inner attributes check?
    match &attr.meta {
        // We ignore the path since it can be from another macro or `#[astarte_aggregate]` without
        // parameters, which we still consider valid.
        syn::Meta::Path(_) => None,
        syn::Meta::NameValue(name) => Some(Err(syn::Error::new(
            name.span(),
            "cannot be used as a named value",
        ))),
        syn::Meta::List(list) => Some(syn::parse2::<T>(list.tokens.clone())),
    }
}

/// Derive macro `#[derive(AstarteAggregate)]` to implement AstarteAggregate.
///
/// ### Example
///
/// ```no_compile
/// #[derive(AstarteAggregate)]
/// struct Foo {
///     bar: String
/// }
/// ```
#[proc_macro_derive(AstarteAggregate, attributes(astarte_aggregate))]
pub fn astarte_aggregate_derive(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let aggregate = parse_macro_input!(input as AggregateDerive);

    // Build the trait implementation
    aggregate.quote().into()
}

/// Derive macro `#[derive(FromEvent)]` to implement the FromEvent trait.
///
/// ### Example
///
/// To derive the trait it for an object.
///
/// ```no_compile
/// #[derive(FromEvent)]
/// #[from_event(interface = "com.example.Foo", path = "obj")]
/// struct Foo {
///     bar: String
/// }
/// ```
///
/// To derive the trait it for an individual.
///
/// ```no_compile
/// #[derive(FromEvent)]
/// #[from_event(interface = "com.example.Sensor", aggregation = "individual")]
/// enum Sensor {
///     #[mapping(endpoint = "/sensor/luminosity")]
///     Luminosity(i32),
///     #[mapping(endpoint = "/sensor/temerature", allow_unset = true)]
///     Temperature(Option<f64>),
/// }
/// ```
#[proc_macro_derive(FromEvent, attributes(from_event, mapping))]
pub fn from_event_derive(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let from_event = parse_macro_input!(input as FromEventDerive);

    // Build the trait implementation
    from_event.quote().into()
}
