//! `#[derive(State)]` 生成 `floatile_sdk::State` trait 实现（schema + initial）。
//!
//! 为每个字段从 Rust 类型推导 JSON Schema 并生成 `Default`。导出面：
//!
//! - `impl State`：`fn schema() -> JsonSchema` + `fn initial() -> Self`
//! - `impl Default`：每个字段用类型默认值

// proc-macro 在编译期展开，输入在宏展开阶段即受信任；unwrap/手动运算可接受。
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unnecessary_map_or,
    clippy::collapsible_if
)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type};

#[proc_macro_derive(State)]
pub fn derive_state(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    match impl_state(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn impl_state(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "State derive 仅支持命名字段的结构体",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(input, "State derive 仅支持结构体"));
        }
    };

    let mut schema_entries = Vec::new();
    let mut field_inits = Vec::new();
    let mut required_fields = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let ty = &field.ty;

        let is_optional = is_option_type(ty);

        let schema_expr = type_to_schema(ty);
        schema_entries.push(quote! {
            (#field_name_str.to_owned(), #schema_expr)
        });

        let init_expr = type_to_default(ty);
        field_inits.push(quote! { #field_name: #init_expr });

        if !is_optional {
            required_fields.push(quote! { #field_name_str.to_owned() });
        }
    }

    Ok(quote! {
        impl #impl_generics floatile_sdk::State for #name #ty_generics #where_clause {
            fn schema() -> floatile_sdk::JsonSchema {
                let mut properties = std::collections::BTreeMap::new();
                #(
                    properties.insert(#schema_entries.0, #schema_entries.1);
                )*
                floatile_sdk::JsonSchema::Object {
                    required: vec![#(#required_fields),*],
                    properties,
                    additional_properties: false,
                }
            }

            fn initial() -> Self {
                Self {
                    #(#field_inits),*
                }
            }
        }

        impl #impl_generics Default for #name #ty_generics #where_clause {
            fn default() -> Self {
                Self::initial()
            }
        }
    })
}

fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(p) = ty {
        p.path
            .segments
            .last()
            .map_or(false, |s| s.ident == "Option")
    } else {
        false
    }
}

/// 从 Rust 类型推导 `floatile_ui_schema::JsonSchema` 表达式（quote 块）。
fn type_to_schema(ty: &Type) -> proc_macro2::TokenStream {
    match ty {
        Type::Path(p) => {
            let last = p.path.segments.last().unwrap();
            let ident = last.ident.to_string();
            match ident.as_str() {
                "String" => quote! {
                    floatile_sdk::JsonSchema::String { max_length: Some(64) }
                },
                "bool" => quote! {
                    floatile_sdk::JsonSchema::Boolean
                },
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => {
                    quote! { floatile_sdk::JsonSchema::Integer }
                }
                "f32" | "f64" => {
                    quote! { floatile_sdk::JsonSchema::Number }
                }
                "Vec" => {
                    if let syn::PathArguments::AngleBracketed(a) = &last.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = a.args.first() {
                            let inner_schema = type_to_schema(inner);
                            return quote! {
                                floatile_sdk::JsonSchema::Array {
                                    max_items: Some(16),
                                    items: Box::new(#inner_schema),
                                }
                            };
                        }
                    }
                    quote! { floatile_sdk::JsonSchema::Array { max_items: Some(16), items: Box::new(floatile_sdk::JsonSchema::String { max_length: None }) } }
                }
                "Option" => {
                    // Option<T>：字段可选（不进 required），schema 按内部类型。
                    if let syn::PathArguments::AngleBracketed(a) = &last.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = a.args.first() {
                            return type_to_schema(inner);
                        }
                    }
                    quote! { floatile_sdk::JsonSchema::String { max_length: Some(64) } }
                }
                _ => {
                    // 嵌套 State：用 State::schema()（该类型必须实现 State）。
                    quote! { <#ty as floatile_sdk::State>::schema() }
                }
            }
        }
        _ => quote! {
            floatile_sdk::JsonSchema::String { max_length: Some(64) }
        },
    }
}

/// 从 Rust 类型推导 `Default::default()` 表达式（用于 State::initial）。
fn type_to_default(ty: &Type) -> proc_macro2::TokenStream {
    match ty {
        Type::Path(p) => {
            let last = p.path.segments.last().unwrap();
            let ident = last.ident.to_string();
            match ident.as_str() {
                "String" => quote! { String::new() },
                "bool" => quote! { false },
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => quote! { 0 },
                "f32" | "f64" => quote! { 0.0 },
                "Vec" => quote! { Vec::new() },
                _ => quote! { Default::default() },
            }
        }
        _ => quote! { Default::default() },
    }
}
