//! SSH profile management: create/edit form + ~/.ssh/config host picker.

mod form;

pub use form::{present as present_ssh_profile_form, present_edit};
