#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub types: Vec<TypeDefinition>,
}

impl Schema {
    pub fn get_type(&self, name: &str) -> Option<&TypeDefinition> {
        self.types.iter().find(|t| t.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDefinition {
    pub name: String,
    pub relations: Vec<RelationDef>,
    pub permissions: Vec<PermissionDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDef {
    pub name: String,
    pub subject_types: Vec<SubjectTypeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectTypeRef {
    pub type_name: String,
    pub relation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDef {
    pub name: String,
    pub rule: RewriteRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteRule {
    This(String),
    Union(Vec<RewriteRule>),
    Intersection(Vec<RewriteRule>),
    Exclusion(Box<RewriteRule>, Box<RewriteRule>),
    Arrow(String, String),
}
