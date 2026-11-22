//! `wire_enum!` (issue #3989): one variant table declares the enum, its
//! checked `TryFrom<repr>` and its `From<Self> for repr`, with the consumer's
//! own error type.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAFruit(u8);

broadcast_common::wire_enum! {
    /// The shape `dvb-si`, `dvb-t2mi` and `dvb-bbframe` use.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fruit: u8, error(NotAFruit), invalid(value) => NotAFruit(value),
    {
        Apple = 1,
        Pear = 2,
    }
}

#[test]
fn one_variant_table_drives_both_conversions() {
    assert_eq!(Fruit::try_from(1), Ok(Fruit::Apple));
    assert_eq!(Fruit::try_from(2), Ok(Fruit::Pear));
    assert_eq!(Fruit::try_from(0), Err(NotAFruit(0)));
    assert_eq!(Fruit::try_from(u8::MAX), Err(NotAFruit(u8::MAX)));
    assert_eq!(u8::from(Fruit::Apple), 1);
    assert_eq!(u8::from(Fruit::Pear), 2);
}
