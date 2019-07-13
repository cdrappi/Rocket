//! A variant of Ident with more ergonomic handling of raw identifiers.

use std::cmp::Ordering;

use devise::syn;
use self::syn::ext::IdentExt as _;
use crate::proc_macro2::{Ident as Ident2, Span};
use quote::ToTokens;

#[repr(transparent)]
#[derive(Clone, Debug, Eq, Ord)]
pub struct Ident(Ident2);

impl Ident {
    pub fn is_raw(&self) -> bool {
        self.0.to_string().starts_with("r#")
    }

    pub fn name(&self) -> String {
        self.0.unraw().to_string()
    }

    pub fn prepend(&self, string: &str) -> Ident {
        // TODO: want to use r#{}{}, but that fails (`"r#__rocket_param_enum"` is not a valid identifier)
        Ident::from(syn::Ident::new(&format!("{}{}", string, self.unraw()), self.span()))
    }

    pub fn span(&self) -> Span {
        self.0.span()
    }
}

impl std::ops::Deref for Ident {
    type Target = Ident2;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Ident {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<Ident2> for Ident {
    fn from(ident: Ident2) -> Ident {
        Ident(ident)
    }
}

impl From<Ident> for Ident2 {
    fn from(ident: Ident) -> Ident2 {
        ident.0
    }
}

impl std::hash::Hash for Ident {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name().hash(state)
    }
}

impl PartialEq for Ident {
    fn eq(&self, other: &Ident) -> bool {
        self.name() == other.name()
    }
}

impl<T: ?Sized + AsRef<str>> PartialEq<T> for Ident {
    fn eq(&self, other: &T) -> bool {
        self.name() == other.as_ref()
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Ident) -> Option<Ordering> {
        self.name().partial_cmp(&other.name())
    }
}

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name().fmt(f)
    }
}

impl syn::parse::Parse for Ident {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::parse::Result<Self> {
        Ident2::parse(input).map(Ident::from)
    }
}

impl ToTokens for Ident {
    fn to_tokens(&self, tokens: &mut crate::proc_macro2::TokenStream) {
        self.0.to_tokens(tokens)
    }
}
