//! A command that makes something and then fails a later step: nothing is made, or what was made
//! is said.

use super::*;

/// A data file to bring in that can't be (its path too long) refuses the account before it's
/// made: it was made, the command failed, and adding it again said "already added".
#[test]
fn an_account_whose_data_file_cant_come_in_isnt_made() {
    let mut s = Session::open(Some(temp("half-done-account")), mailcraft_model::dates::Clock::new(NOW, 0));
    let r = s.run("account.add", &json!({"email": "zoe@studio.example", "local": true, "pst": "x".repeat(10_000)}));
    assert!(matches!(&r, Err(CmdError::Params(_))), "{r:?}");
    assert!(s.accounts.is_empty(), "nothing made");
    s.run("account.add", &json!({"email": "zoe@studio.example", "local": true})).unwrap();
}
