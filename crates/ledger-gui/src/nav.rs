//! Where you are and how you got there: a browser-style history.

use ledger_core::{AccountId, AssetId, EntryId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReportKind {
    ProfitAndLoss,
    BalanceSheet,
    CashFlow,
    TrialBalance,
    GeneralLedger,
    MembersEquity,
    TaxSummary,
    ArAging,
    ApAging,
    Vendor1099,
    FixedAssetRegister,
}

impl ReportKind {
    pub const ALL: [ReportKind; 11] = [
        ReportKind::ProfitAndLoss,
        ReportKind::BalanceSheet,
        ReportKind::CashFlow,
        ReportKind::MembersEquity,
        ReportKind::TrialBalance,
        ReportKind::GeneralLedger,
        ReportKind::FixedAssetRegister,
        ReportKind::TaxSummary,
        ReportKind::Vendor1099,
        ReportKind::ArAging,
        ReportKind::ApAging,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ReportKind::ProfitAndLoss => "Profit and loss",
            ReportKind::BalanceSheet => "Balance sheet",
            ReportKind::CashFlow => "Cash flow",
            ReportKind::TrialBalance => "Trial balance",
            ReportKind::GeneralLedger => "General ledger",
            ReportKind::MembersEquity => "Owners' equity",
            ReportKind::TaxSummary => "Tax summary",
            ReportKind::ArAging => "Receivables aging",
            ReportKind::ApAging => "Payables aging",
            ReportKind::Vendor1099 => "1099 worksheet",
            ReportKind::FixedAssetRegister => "Fixed asset register",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Overview,
    Cash,
    Profit,
    Transactions,
    Entry(EntryId),
    Reports(ReportKind),
    Accounts,
    Account(AccountId),
    /// The fixed asset register.
    Assets,
    /// One fixed asset.
    Asset(AssetId),
    Documents,
    Document(u64),
    Company,
    Journal,
    JournalEvent(u64),
}

impl Route {
    /// The sidebar section this route belongs to.
    pub fn section(&self) -> Section {
        match self {
            Route::Overview => Section::Overview,
            Route::Cash => Section::Cash,
            Route::Profit => Section::Profit,
            Route::Transactions | Route::Entry(_) => Section::Transactions,
            Route::Reports(_) => Section::Reports,
            Route::Accounts | Route::Account(_) => Section::Accounts,
            Route::Assets | Route::Asset(_) => Section::Assets,
            Route::Documents | Route::Document(_) => Section::Documents,
            Route::Company => Section::Company,
            Route::Journal | Route::JournalEvent(_) => Section::Journal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Cash,
    Profit,
    Transactions,
    Reports,
    Accounts,
    Assets,
    Documents,
    Company,
    Journal,
}

impl Section {
    pub const ALL: [Section; 10] = [
        Section::Overview,
        Section::Cash,
        Section::Profit,
        Section::Transactions,
        Section::Reports,
        Section::Accounts,
        Section::Assets,
        Section::Documents,
        Section::Company,
        Section::Journal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Cash => "Cash",
            Section::Profit => "Profit",
            Section::Transactions => "Transactions",
            Section::Reports => "Reports",
            Section::Accounts => "Accounts",
            Section::Assets => "Assets",
            Section::Documents => "Documents",
            Section::Company => "Company",
            Section::Journal => "Journal",
        }
    }

    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as i;
        match self {
            Section::Overview => i::SQUARES_FOUR,
            Section::Cash => i::BANK,
            Section::Profit => i::TREND_UP,
            Section::Transactions => i::LIST_BULLETS,
            Section::Reports => i::FILE_TEXT,
            Section::Accounts => i::TREE_STRUCTURE,
            Section::Assets => i::PACKAGE,
            Section::Documents => i::FILES,
            Section::Company => i::BUILDINGS,
            Section::Journal => i::SCROLL,
        }
    }

    pub fn home(self) -> Route {
        match self {
            Section::Overview => Route::Overview,
            Section::Cash => Route::Cash,
            Section::Profit => Route::Profit,
            Section::Transactions => Route::Transactions,
            Section::Reports => Route::Reports(ReportKind::ProfitAndLoss),
            Section::Accounts => Route::Accounts,
            Section::Assets => Route::Assets,
            Section::Documents => Route::Documents,
            Section::Company => Route::Company,
            Section::Journal => Route::Journal,
        }
    }
}

/// ```text
/// GOAL: back and forward behave like a web browser's.
/// SOURCE: CONVENTION (browser session history: going somewhere new
///         drops the forward list).
/// INVARIANTS:
///   1. go(r) to a different route makes it current, pushes the old one
///      on back, and clears forward; go to the current route does nothing.
///   2. back() then forward() returns to the same route (and vice versa);
///      either does nothing when its list is empty.
///   3. can_back/can_forward say whether back/forward would move.
/// ```
/// Back/forward history.
pub struct Nav {
    back: Vec<Route>,
    forward: Vec<Route>,
    pub current: Route,
}

impl Nav {
    pub fn new(start: Route) -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            current: start,
        }
    }

    pub fn go(&mut self, to: Route) {
        if to != self.current {
            self.back.push(std::mem::replace(&mut self.current, to));
            self.forward.clear();
        }
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn back(&mut self) {
        if let Some(r) = self.back.pop() {
            self.forward.push(std::mem::replace(&mut self.current, r));
        }
    }

    pub fn forward(&mut self) {
        if let Some(r) = self.forward.pop() {
            self.back.push(std::mem::replace(&mut self.current, r));
        }
    }
}
