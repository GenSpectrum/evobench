use std::sync::Arc;

use noisy_float::prelude::*;
use serde::{Deserialize, Serialize};

use crate::utillib::arc::CloneArc;

macro_rules! copy {
    { $n:ident } => {
        #[allow(non_snake_case)]
        let $n = *$n;
    }
}

// macro_rules! clone_arc {
//     { $n:ident } => {
//         #[allow(non_snake_case)]
//         let $n = crate::utillib::arc::CloneArc::clone_arc($n);
//     }
// }

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DayDate(pub String);

// pub enum Column {
//     PangoLineage, // pangoLineage
// }

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct Column(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(non_snake_case)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum FilterExpression {
    Or {
        children: Vec<Arc<FilterExpression>>,
    },
    And {
        children: Vec<Arc<FilterExpression>>,
    },
    Not {
        child: Arc<FilterExpression>,
    },
    #[serde(rename = "N-Of")]
    NOf {
        children: Vec<Arc<FilterExpression>>,
        matchExactly: bool,
        numberOfMatchers: usize,
    },

    DateBetween {
        from: Option<DayDate>,
        to: Option<DayDate>,
        column: Column,
    },
    Lineage {
        column: Column,
        includeSublineages: bool,
        value: String,
    },
    StringEquals {
        column: Column,
        value: String,
    },
    True {},
    NucleotideEquals {
        symbol: String,
        position: usize,
    },
    HasNucleotideMutation {
        position: usize,
    },
    HasAminoAcidMutation {
        position: usize,
        sequenceName: String,
    },
    FloatBetween {
        column: Column,
        from: N64,
        to: N64,
    },
    AminoAcidInsertionContains {
        position: usize,
        sequenceName: String, // what kind of?
        value: String,        // what kind of?
    },
    AminoAcidEquals {
        position: usize,
        sequenceName: String, //
        symbol: String,       // can be "*"
    },
}

impl FilterExpression {
    pub fn is_not(&self) -> Option<&Arc<FilterExpression>> {
        match self {
            FilterExpression::Not { child } => Some(child),
            _ => None,
        }
    }

    pub fn is_or(&self) -> Option<&Vec<Arc<FilterExpression>>> {
        match self {
            FilterExpression::Or { children } => Some(children),
            _ => None,
        }
    }

    pub fn is_and(&self) -> Option<&Vec<Arc<FilterExpression>>> {
        match self {
            FilterExpression::And { children } => Some(children),
            _ => None,
        }
    }

    pub fn optimize(self: &Arc<Self>) -> Arc<FilterExpression> {
        match &**self {
            FilterExpression::Or { children } => {
                if children.len() == 1 {
                    children[0].clone_arc().optimize()
                } else {
                    let mut new_children = Vec::new();
                    for child in children {
                        let child = child.optimize();
                        if let Some(subchildren) = child.is_or() {
                            for child in subchildren {
                                // already optimized
                                new_children.push(child.clone_arc());
                            }
                        } else {
                            new_children.push(child);
                        }
                    }
                    FilterExpression::Or {
                        children: new_children,
                    }
                    .into()
                }
            }
            FilterExpression::And { children } => {
                if children.len() == 1 {
                    children[0].clone_arc().optimize()
                } else {
                    let mut new_children = Vec::new();
                    for child in children {
                        let child = child.optimize();
                        if let Some(subchildren) = child.is_and() {
                            for child in subchildren {
                                // already optimized
                                new_children.push(child.clone_arc());
                            }
                        } else {
                            new_children.push(child);
                        }
                    }
                    FilterExpression::And {
                        children: new_children,
                    }
                    .into()
                }
            }
            FilterExpression::Not { child } => {
                let child = child.optimize();
                if let Some(child_child) = child.is_not() {
                    child_child.clone_arc()
                } else {
                    FilterExpression::Not { child }.into()
                }
            }
            FilterExpression::NOf {
                children,
                matchExactly,
                numberOfMatchers,
            } => {
                let children = children.iter().map(|e| e.optimize()).collect();
                copy!(matchExactly);
                copy!(numberOfMatchers);
                FilterExpression::NOf {
                    children,
                    matchExactly,
                    numberOfMatchers,
                }
                .into()
            }

            // Non-recursive cases, not optimizable
            FilterExpression::DateBetween {
                from: _,
                to: _,
                column: _,
            } => self.clone_arc(),
            FilterExpression::Lineage {
                column: _,
                includeSublineages: _,
                value: _,
            } => self.clone_arc(),
            FilterExpression::StringEquals {
                column: _,
                value: _,
            } => self.clone_arc(),
            FilterExpression::True {} => self.clone_arc(),
            FilterExpression::NucleotideEquals {
                symbol: _,
                position: _,
            } => self.clone_arc(),
            FilterExpression::HasNucleotideMutation { position: _ } => self.clone_arc(),
            FilterExpression::HasAminoAcidMutation {
                position: _,
                sequenceName: _,
            } => self.clone_arc(),
            FilterExpression::FloatBetween {
                column: _,
                from: _,
                to: _,
            } => self.clone_arc(),
            FilterExpression::AminoAcidInsertionContains {
                position: _,
                sequenceName: _,
                value: _,
            } => self.clone_arc(),
            FilterExpression::AminoAcidEquals {
                position: _,
                sequenceName: _,
                symbol: _,
            } => self.clone_arc(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(non_snake_case)]
#[serde(deny_unknown_fields)]
pub enum Order {
    #[serde(rename = "ascending")]
    Ascending,
    #[serde(rename = "descending")]
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(non_snake_case)]
#[serde(deny_unknown_fields)]
pub struct FieldOrder {
    field: String,
    order: Order,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(non_snake_case)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum Action {
    AminoAcidMutations {
        minProportion: N64,
        randomize: bool,
        orderByFields: Option<Vec<FieldOrder>>,
        limit: Option<usize>,
    },
    Details {
        fields: Vec<String>,
        randomize: bool,
    },
    Mutations {
        minProportion: N64,
        randomize: bool,
        limit: Option<usize>,
    },
    Aggregated {
        groupByFields: Option<Vec<String>>, // non-empty ones?
        randomize: bool,
        orderByFields: Option<Vec<FieldOrder>>,
        limit: Option<usize>,
    },
    AminoAcidInsertions {
        randomize: bool,
    },
    FastaAligned {
        randomize: bool,
        sequenceName: String, // "main" und so
    },
    Insertions {
        randomize: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(non_snake_case)]
pub struct Query {
    pub action: Arc<Action>,
    pub filterExpression: Arc<FilterExpression>,
}

impl Query {
    pub fn optimize(&self) -> Query {
        let Query {
            action,
            filterExpression,
        } = self;
        let action = action.clone_arc();
        #[allow(non_snake_case)]
        let filterExpression = filterExpression.optimize();
        Query {
            action,
            filterExpression,
        }
    }
}
