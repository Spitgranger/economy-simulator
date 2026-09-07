//! The sectors. Three goods that homes buy daily, plus education, whose
//! output is school seats sold monthly (private) or provided publicly.

pub const NG: usize = 4;
pub const FOOD: usize = 0;
pub const SHELTER: usize = 1;
pub const LUXURY: usize = 2;
pub const EDUCATION: usize = 3;

pub struct GoodSpec {
    pub name: &'static str,
    /// units per effective (skill-weighted) worker per day (goods only)
    pub productivity: f64,
    pub init_price: i64,
    /// lowest skill a firm in this sector will hire
    pub min_skill: f64,
    /// weight in the consumer price index: typical daily units per home
    pub index_weight: f64,
    /// initial share of firms in this sector
    pub firm_share: f64,
}

pub const GOODS: [GoodSpec; NG] = [
    GoodSpec { name: "food", productivity: 6.8, init_price: 50, min_skill: 0.5, index_weight: 2.4, firm_share: 0.45 },
    GoodSpec { name: "shelter", productivity: 4.2, init_price: 80, min_skill: 0.7, index_weight: 1.0, firm_share: 0.35 },
    GoodSpec { name: "luxury", productivity: 1.7, init_price: 200, min_skill: 1.0, index_weight: 0.2, firm_share: 0.20 },
    GoodSpec { name: "education", productivity: 1.0, init_price: 800, min_skill: 1.0, index_weight: 0.0, firm_share: 0.0 },
];

/// Purchase order in the daily goods market: necessities first, luxury from what is left.
pub const PRIORITY: [usize; 3] = [SHELTER, FOOD, LUXURY];

/// Pupils per teacher in a private school; a school's seats per month = teachers x this.
pub const PRIVATE_CLASS_SIZE: u32 = 15;
