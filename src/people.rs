//! People, structure-of-arrays. A person has an age, a skill, a job and
//! political preferences, and belongs to exactly one home (`home`), which
//! holds the money. Slots of the dead are recycled through a free list.
//!
//! Life course: minor until 18 (in school 6-17), worker until 65, retired after.

pub const NO_FIRM: u32 = u32::MAX;
pub const NO_PARENT: u32 = u32::MAX;
pub const NO_HOME: u32 = u32::MAX;
pub const ADULT_AGE: u32 = 18 * 12;
pub const RETIRE_AGE: u32 = 65 * 12;
pub const SCHOOL_START: u32 = 6 * 12;

#[derive(Clone)]
pub struct PersonInit {
    pub age_months: u32,
    pub parent: u32,
    pub home: u32,
    pub skill: f64,
    pub appetite: f64,
    pub shelter_need: f64,
    pub status: f64,
    pub patience: f64,
    pub price_sens: f64,
    pub leisure: f64,
    pub risk: f64,
    pub search: f64,
    pub family: f64,
    pub reservation_wage: i64,
    pub pref: f64,
}

pub struct People {
    pub n: usize,
    pub alive: Vec<bool>,
    pub age_months: Vec<u32>,
    pub parent: Vec<u32>,
    pub home: Vec<u32>,
    pub retired: Vec<bool>,
    pub children_born: Vec<u8>,
    pub hc: Vec<f64>,             // human capital accumulated in school, 0..1
    pub private_school: Vec<bool>, // enrolled privately this month (stats)
    pub school: Vec<u32>,          // private school attended last month, NO_FIRM = public or none

    pub skill: Vec<f64>,
    pub employer: Vec<u32>,
    pub pay: Vec<i64>,
    pub reservation_wage: Vec<i64>,
    pub unemployed_weeks: Vec<u32>,

    pub pref: Vec<f64>,
    pub pref_prior: Vec<f64>,
    pub real_income_at_election: Vec<f64>,

    pub appetite: Vec<f64>,
    pub shelter_need: Vec<f64>,
    pub status: Vec<f64>,
    pub patience: Vec<f64>,
    pub price_sens: Vec<f64>,
    pub leisure: Vec<f64>,
    pub risk: Vec<f64>,
    pub search: Vec<f64>,
    pub family: Vec<f64>,

    free: Vec<u32>,
}

impl People {
    pub fn with_capacity(n: usize) -> People {
        macro_rules! cols { ($($f:ident),*) => { People { n: 0, free: Vec::new(), $($f: Vec::with_capacity(n)),* } } }
        cols!(
            alive, age_months, parent, home, retired, children_born, hc, private_school, school, skill, employer, pay,
            reservation_wage, unemployed_weeks, pref, pref_prior, real_income_at_election, appetite, shelter_need,
            status, patience, price_sens, leisure, risk, search, family
        )
    }

    fn append_defaults(&mut self) -> usize {
        self.alive.push(false);
        self.age_months.push(0);
        self.parent.push(NO_PARENT);
        self.home.push(NO_HOME);
        self.retired.push(false);
        self.children_born.push(0);
        self.hc.push(0.0);
        self.private_school.push(false);
        self.school.push(NO_FIRM);
        self.skill.push(0.0);
        self.employer.push(NO_FIRM);
        self.pay.push(0);
        self.reservation_wage.push(0);
        self.unemployed_weeks.push(0);
        self.pref.push(0.5);
        self.pref_prior.push(0.5);
        self.real_income_at_election.push(0.0);
        self.appetite.push(0.0);
        self.shelter_need.push(0.0);
        self.status.push(0.0);
        self.patience.push(1.0);
        self.price_sens.push(20.0);
        self.leisure.push(1.0);
        self.risk.push(0.5);
        self.search.push(0.5);
        self.family.push(1.0);
        self.n += 1;
        self.n - 1
    }

    pub fn write(&mut self, i: usize, p: PersonInit) {
        self.alive[i] = true;
        self.age_months[i] = p.age_months;
        self.parent[i] = p.parent;
        self.home[i] = p.home;
        self.retired[i] = p.age_months >= RETIRE_AGE;
        self.children_born[i] = 0;
        self.hc[i] = 0.0;
        self.private_school[i] = false;
        self.school[i] = NO_FIRM;
        self.skill[i] = p.skill;
        self.employer[i] = NO_FIRM;
        self.pay[i] = 0;
        self.reservation_wage[i] = p.reservation_wage;
        self.unemployed_weeks[i] = 0;
        self.pref[i] = p.pref;
        self.pref_prior[i] = p.pref;
        self.real_income_at_election[i] = 0.0;
        self.appetite[i] = p.appetite;
        self.shelter_need[i] = p.shelter_need;
        self.status[i] = p.status;
        self.patience[i] = p.patience;
        self.price_sens[i] = p.price_sens;
        self.leisure[i] = p.leisure;
        self.risk[i] = p.risk;
        self.search[i] = p.search;
        self.family[i] = p.family;
    }

    pub fn spawn(&mut self, p: PersonInit) -> usize {
        let i = match self.free.pop() {
            Some(i) => i as usize,
            None => self.append_defaults(),
        };
        self.write(i, p);
        i
    }

    pub fn bury(&mut self, i: usize) {
        self.alive[i] = false;
        self.employer[i] = NO_FIRM;
        self.home[i] = NO_HOME;
        self.retired[i] = false;
        self.free.push(i as u32);
    }

    #[inline]
    pub fn is_employed(&self, i: usize) -> bool {
        self.employer[i] != NO_FIRM
    }

    #[inline]
    pub fn is_adult(&self, i: usize) -> bool {
        self.alive[i] && self.age_months[i] >= ADULT_AGE
    }

    #[inline]
    pub fn is_worker(&self, i: usize) -> bool {
        self.is_adult(i) && !self.retired[i]
    }

    #[inline]
    pub fn is_pupil(&self, i: usize) -> bool {
        self.alive[i] && self.age_months[i] >= SCHOOL_START && self.age_months[i] < ADULT_AGE
    }
}
