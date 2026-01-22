use anyhow::Result;
use ctx_core::CtxRepo;

/// Declarative assertions on CTX state
pub enum Assertion {
    // Session state
    SessionState(SessionStateMatch),
    NoSession,
    SessionExists,

    // Commits
    CommitCount(usize),
    CommitCountGte(usize),
    HeadMessageContains(String),

    // Files
    FileInHead {
        path: String,
    },
    FileContentContains {
        path: String,
        content: String,
    },
    FileNotInHead {
        path: String,
    },

    // Staging
    StagingExists,
    NoStaging,
    StagingChainLengthGte(usize),
    StagingContainsFile {
        path: String,
    },
    StagingContainsNote {
        text: String,
    },

    // Narrative
    NoteContains(String),

    // Graph/Edges
    EdgeExists {
        from: String,
        to: String,
        label: String,
    },

    // Query/Retrieval
    QueryReturnsPath {
        query: String,
        path: String,
    },
    QueryTokensWithinBudget {
        query: String,
        budget: usize,
    },

    // Recovery
    SessionRecovered,
    NoPanic,

    // Custom (takes mutable reference to allow mutations)
    Custom(Box<dyn Fn(&mut CtxRepo) -> Result<()> + Send + Sync>),
}

impl std::fmt::Debug for Assertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionState(s) => write!(f, "SessionState({:?})", s),
            Self::NoSession => write!(f, "NoSession"),
            Self::SessionExists => write!(f, "SessionExists"),
            Self::CommitCount(n) => write!(f, "CommitCount({})", n),
            Self::CommitCountGte(n) => write!(f, "CommitCountGte({})", n),
            Self::HeadMessageContains(s) => write!(f, "HeadMessageContains({:?})", s),
            Self::FileInHead { path } => write!(f, "FileInHead {{ path: {:?} }}", path),
            Self::FileContentContains { path, content } => {
                write!(f, "FileContentContains {{ path: {:?}, content: {:?} }}", path, content)
            }
            Self::FileNotInHead { path } => write!(f, "FileNotInHead {{ path: {:?} }}", path),
            Self::StagingExists => write!(f, "StagingExists"),
            Self::NoStaging => write!(f, "NoStaging"),
            Self::StagingChainLengthGte(n) => write!(f, "StagingChainLengthGte({})", n),
            Self::StagingContainsFile { path } => {
                write!(f, "StagingContainsFile {{ path: {:?} }}", path)
            }
            Self::StagingContainsNote { text } => {
                write!(f, "StagingContainsNote {{ text: {:?} }}", text)
            }
            Self::NoteContains(s) => write!(f, "NoteContains({:?})", s),
            Self::EdgeExists { from, to, label } => {
                write!(f, "EdgeExists {{ from: {:?}, to: {:?}, label: {:?} }}", from, to, label)
            }
            Self::QueryReturnsPath { query, path } => {
                write!(f, "QueryReturnsPath {{ query: {:?}, path: {:?} }}", query, path)
            }
            Self::QueryTokensWithinBudget { query, budget } => {
                write!(
                    f,
                    "QueryTokensWithinBudget {{ query: {:?}, budget: {} }}",
                    query, budget
                )
            }
            Self::SessionRecovered => write!(f, "SessionRecovered"),
            Self::NoPanic => write!(f, "NoPanic"),
            Self::Custom(_) => write!(f, "Custom(<fn>)"),
        }
    }
}

/// Match against session states
#[derive(Clone, Debug)]
pub enum SessionStateMatch {
    Running,
    AwaitingUser,
    Interrupted,
    PendingComplete,
    Complete,
    Aborted,
}
