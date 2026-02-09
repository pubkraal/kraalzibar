use pest::Parser;
use pest_derive::Parser;

use super::types::{
    PermissionDef, RelationDef, RewriteRule, Schema, SubjectTypeRef, TypeDefinition,
};

#[derive(Parser)]
#[grammar = "schema/grammar.pest"]
struct SchemaParser;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("syntax error: {0}")]
    Syntax(String),
    #[error("mixed operators in permission expression: use only one of +, &, - per expression")]
    MixedOperators,
    #[error("exclusion (-) supports exactly two operands: base - excluded")]
    MultipleExclusions,
    #[error("duplicate type: {0}")]
    DuplicateType(String),
    #[error("duplicate relation '{relation}' in type '{type_name}'")]
    DuplicateRelation { type_name: String, relation: String },
    #[error("duplicate permission '{permission}' in type '{type_name}'")]
    DuplicatePermission {
        type_name: String,
        permission: String,
    },
}

pub fn parse_schema(input: &str) -> Result<Schema, ParseError> {
    let pairs =
        SchemaParser::parse(Rule::schema, input).map_err(|e| ParseError::Syntax(e.to_string()))?;

    let mut types = Vec::new();
    let mut seen_types = std::collections::HashSet::new();

    for pair in pairs {
        if pair.as_rule() != Rule::schema {
            continue;
        }
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::definition {
                let type_def = parse_definition(inner)?;
                if !seen_types.insert(type_def.name.clone()) {
                    return Err(ParseError::DuplicateType(type_def.name));
                }
                types.push(type_def);
            }
        }
    }

    Ok(Schema { types })
}

fn unexpected_rule(rule: Rule) -> ParseError {
    ParseError::Syntax(format!("unexpected rule: {rule:?}"))
}

fn missing_token(context: &str) -> ParseError {
    ParseError::Syntax(format!("missing token: {context}"))
}

fn parse_definition(pair: pest::iterators::Pair<'_, Rule>) -> Result<TypeDefinition, ParseError> {
    let mut inner = pair.into_inner();
    let name = inner
        .next()
        .ok_or_else(|| missing_token("definition name"))?
        .as_str()
        .to_string();
    let body = inner
        .next()
        .ok_or_else(|| missing_token("definition body"))?;

    let mut relations = Vec::new();
    let mut permissions = Vec::new();
    let mut seen_relations = std::collections::HashSet::new();
    let mut seen_permissions = std::collections::HashSet::new();

    for item in body.into_inner() {
        match item.as_rule() {
            Rule::relation_def => {
                let rel = parse_relation_def(item)?;
                if !seen_relations.insert(rel.name.clone()) {
                    return Err(ParseError::DuplicateRelation {
                        type_name: name,
                        relation: rel.name,
                    });
                }
                relations.push(rel);
            }
            Rule::permission_def => {
                let perm = parse_permission_def(item)?;
                if !seen_permissions.insert(perm.name.clone()) {
                    return Err(ParseError::DuplicatePermission {
                        type_name: name,
                        permission: perm.name,
                    });
                }
                permissions.push(perm);
            }
            _ => {}
        }
    }

    Ok(TypeDefinition {
        name,
        relations,
        permissions,
    })
}

fn parse_relation_def(pair: pest::iterators::Pair<'_, Rule>) -> Result<RelationDef, ParseError> {
    let mut inner = pair.into_inner();
    let name = inner
        .next()
        .ok_or_else(|| missing_token("relation name"))?
        .as_str()
        .to_string();
    let subject_type_list = inner
        .next()
        .ok_or_else(|| missing_token("subject type list"))?;

    let subject_types = subject_type_list
        .into_inner()
        .map(parse_subject_type_ref)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(RelationDef {
        name,
        subject_types,
    })
}

fn parse_subject_type_ref(
    pair: pest::iterators::Pair<'_, Rule>,
) -> Result<SubjectTypeRef, ParseError> {
    let mut inner = pair.into_inner();
    let type_name = inner
        .next()
        .ok_or_else(|| missing_token("subject type name"))?
        .as_str()
        .to_string();
    let relation = inner.next().map(|p| p.as_str().to_string());

    Ok(SubjectTypeRef {
        type_name,
        relation,
    })
}

fn parse_permission_def(
    pair: pest::iterators::Pair<'_, Rule>,
) -> Result<PermissionDef, ParseError> {
    let mut inner = pair.into_inner();
    let name = inner
        .next()
        .ok_or_else(|| missing_token("permission name"))?
        .as_str()
        .to_string();
    let expr = inner
        .next()
        .ok_or_else(|| missing_token("permission expression"))?;
    let rule = parse_permission_expr(expr)?;

    Ok(PermissionDef { name, rule })
}

fn parse_permission_expr(pair: pest::iterators::Pair<'_, Rule>) -> Result<RewriteRule, ParseError> {
    let mut inner = pair.into_inner();
    let first = parse_permission_term(
        inner
            .next()
            .ok_or_else(|| missing_token("permission term"))?,
    )?;

    let mut ops_and_terms: Vec<(Rule, RewriteRule)> = Vec::new();

    while let Some(op) = inner.next() {
        let term = parse_permission_term(
            inner
                .next()
                .ok_or_else(|| missing_token("permission term after operator"))?,
        )?;
        ops_and_terms.push((op.as_rule(), term));
    }

    if ops_and_terms.is_empty() {
        return Ok(first);
    }

    let first_op = ops_and_terms[0].0;

    let has_mixed = ops_and_terms.iter().any(|(op, _)| *op != first_op);
    if has_mixed {
        return Err(ParseError::MixedOperators);
    }

    match first_op {
        Rule::union_op => {
            let mut children = vec![first];
            children.extend(ops_and_terms.into_iter().map(|(_, t)| t));
            Ok(RewriteRule::Union(children))
        }
        Rule::intersection_op => {
            let mut children = vec![first];
            children.extend(ops_and_terms.into_iter().map(|(_, t)| t));
            Ok(RewriteRule::Intersection(children))
        }
        Rule::exclusion_op => {
            if ops_and_terms.len() != 1 {
                return Err(ParseError::MultipleExclusions);
            }
            let (_, subtract) = ops_and_terms
                .into_iter()
                .next()
                .ok_or_else(|| missing_token("exclusion operand"))?;
            Ok(RewriteRule::Exclusion(Box::new(first), Box::new(subtract)))
        }
        _ => Err(unexpected_rule(first_op)),
    }
}

fn parse_permission_term(pair: pest::iterators::Pair<'_, Rule>) -> Result<RewriteRule, ParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| missing_token("permission term content"))?;
    match inner.as_rule() {
        Rule::arrow_expr => {
            let mut parts = inner.into_inner();
            let tupleset = parts
                .next()
                .ok_or_else(|| missing_token("arrow tupleset"))?
                .as_str()
                .to_string();
            let computed = parts
                .next()
                .ok_or_else(|| missing_token("arrow computed"))?
                .as_str()
                .to_string();
            Ok(RewriteRule::Arrow(tupleset, computed))
        }
        Rule::identifier => Ok(RewriteRule::This(inner.as_str().to_string())),
        other => Err(unexpected_rule(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_definition() {
        let schema = parse_schema("definition user {}").unwrap();

        assert_eq!(schema.types.len(), 1);
        assert_eq!(schema.types[0].name, "user");
        assert!(schema.types[0].relations.is_empty());
        assert!(schema.types[0].permissions.is_empty());
    }

    #[test]
    fn parse_single_direct_relation() {
        let schema = parse_schema("definition document { relation owner: user }").unwrap();

        let doc = schema.get_type("document").unwrap();
        assert_eq!(doc.relations.len(), 1);
        assert_eq!(doc.relations[0].name, "owner");
        assert_eq!(doc.relations[0].subject_types.len(), 1);
        assert_eq!(doc.relations[0].subject_types[0].type_name, "user");
        assert_eq!(doc.relations[0].subject_types[0].relation, None);
    }

    #[test]
    fn parse_userset_subject_type() {
        let schema =
            parse_schema("definition group { relation member: user | group#member }").unwrap();

        let group = schema.get_type("group").unwrap();
        let member = &group.relations[0];
        assert_eq!(member.subject_types.len(), 2);
        assert_eq!(member.subject_types[0].type_name, "user");
        assert_eq!(member.subject_types[0].relation, None);
        assert_eq!(member.subject_types[1].type_name, "group");
        assert_eq!(member.subject_types[1].relation, Some("member".to_string()));
    }

    #[test]
    fn parse_union_permission() {
        let schema = parse_schema(
            "definition doc { relation owner: user relation editor: user permission can_edit = owner + editor }",
        )
        .unwrap();

        let doc = schema.get_type("doc").unwrap();
        let perm = &doc.permissions[0];
        assert_eq!(perm.name, "can_edit");
        assert_eq!(
            perm.rule,
            RewriteRule::Union(vec![
                RewriteRule::This("owner".to_string()),
                RewriteRule::This("editor".to_string()),
            ])
        );
    }

    #[test]
    fn parse_intersection_permission() {
        let schema = parse_schema(
            "definition doc { relation owner: user relation reviewer: user permission can_approve = owner & reviewer }",
        )
        .unwrap();

        let doc = schema.get_type("doc").unwrap();
        let perm = &doc.permissions[0];
        assert_eq!(
            perm.rule,
            RewriteRule::Intersection(vec![
                RewriteRule::This("owner".to_string()),
                RewriteRule::This("reviewer".to_string()),
            ])
        );
    }

    #[test]
    fn parse_exclusion_permission() {
        let schema = parse_schema(
            "definition doc { relation viewer: user relation banned: user permission can_view = viewer - banned }",
        )
        .unwrap();

        let doc = schema.get_type("doc").unwrap();
        let perm = &doc.permissions[0];
        assert_eq!(
            perm.rule,
            RewriteRule::Exclusion(
                Box::new(RewriteRule::This("viewer".to_string())),
                Box::new(RewriteRule::This("banned".to_string())),
            )
        );
    }

    #[test]
    fn parse_arrow_permission() {
        let schema = parse_schema(
            "definition object { relation parent: category permission can_view = parent->can_view }",
        )
        .unwrap();

        let obj = schema.get_type("object").unwrap();
        let perm = &obj.permissions[0];
        assert_eq!(
            perm.rule,
            RewriteRule::Arrow("parent".to_string(), "can_view".to_string())
        );
    }

    #[test]
    fn parse_union_with_arrow() {
        let schema = parse_schema(
            "definition object { relation viewer: user relation parent: category permission can_view = viewer + parent->can_view }",
        )
        .unwrap();

        let obj = schema.get_type("object").unwrap();
        let perm = &obj.permissions[0];
        assert_eq!(
            perm.rule,
            RewriteRule::Union(vec![
                RewriteRule::This("viewer".to_string()),
                RewriteRule::Arrow("parent".to_string(), "can_view".to_string()),
            ])
        );
    }

    #[test]
    fn parse_multiple_definitions() {
        let input = r#"
            definition user {}
            definition group {
                relation member: user
            }
        "#;
        let schema = parse_schema(input).unwrap();

        assert_eq!(schema.types.len(), 2);
        assert_eq!(schema.types[0].name, "user");
        assert_eq!(schema.types[1].name, "group");
    }

    #[test]
    fn parse_full_example_from_plan() {
        let input = r#"
            definition user {}

            definition group {
                relation member: user | group#member
            }

            definition category {
                relation owner: user
                relation editor: user | group#member
                relation viewer: user | group#member

                permission can_edit = owner + editor
                permission can_view = can_edit + viewer
            }

            definition object {
                relation parent: category
                relation owner: user
                relation editor: user | group#member
                relation viewer: user | group#member

                permission can_edit = owner + editor + parent->can_edit
                permission can_view = can_edit + viewer + parent->can_view
            }
        "#;
        let schema = parse_schema(input).unwrap();

        assert_eq!(schema.types.len(), 4);

        let category = schema.get_type("category").unwrap();
        assert_eq!(category.relations.len(), 3);
        assert_eq!(category.permissions.len(), 2);

        let object = schema.get_type("object").unwrap();
        assert_eq!(object.relations.len(), 4);
        assert_eq!(object.permissions.len(), 2);

        let can_edit = &object.permissions[0];
        assert_eq!(
            can_edit.rule,
            RewriteRule::Union(vec![
                RewriteRule::This("owner".to_string()),
                RewriteRule::This("editor".to_string()),
                RewriteRule::Arrow("parent".to_string(), "can_edit".to_string()),
            ])
        );
    }

    #[test]
    fn reject_mixed_operators() {
        let input = "definition doc { relation a: user relation b: user relation c: user permission p = a + b & c }";
        let result = parse_schema(input);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ParseError::MixedOperators);
    }

    #[test]
    fn parse_empty_schema() {
        let schema = parse_schema("").unwrap();

        assert!(schema.types.is_empty());
    }

    #[test]
    fn parse_comments_and_whitespace() {
        let input = r#"
            // This is a comment
            definition user {}

            // Another comment
            definition group {
                // relation comment
                relation member: user
            }
        "#;
        let schema = parse_schema(input).unwrap();

        assert_eq!(schema.types.len(), 2);
    }

    #[test]
    fn invalid_syntax_produces_error() {
        let result = parse_schema("not valid syntax at all");

        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::Syntax(msg) => {
                assert!(!msg.is_empty());
            }
            other => panic!("expected Syntax error, got: {other:?}"),
        }
    }

    #[test]
    fn reject_multiple_exclusion_operators() {
        let input = "definition doc { relation a: user relation b: user relation c: user permission p = a - b - c }";
        let result = parse_schema(input);

        assert_eq!(result.unwrap_err(), ParseError::MultipleExclusions);
    }

    #[test]
    fn reject_duplicate_type() {
        let input = "definition user {} definition user {}";
        let result = parse_schema(input);

        assert_eq!(
            result.unwrap_err(),
            ParseError::DuplicateType("user".to_string())
        );
    }

    #[test]
    fn reject_duplicate_relation() {
        let input = "definition doc { relation owner: user relation owner: user }";
        let result = parse_schema(input);

        assert_eq!(
            result.unwrap_err(),
            ParseError::DuplicateRelation {
                type_name: "doc".to_string(),
                relation: "owner".to_string(),
            }
        );
    }

    #[test]
    fn reject_duplicate_permission() {
        let input = "definition doc { relation owner: user permission can_edit = owner permission can_edit = owner }";
        let result = parse_schema(input);

        assert_eq!(
            result.unwrap_err(),
            ParseError::DuplicatePermission {
                type_name: "doc".to_string(),
                permission: "can_edit".to_string(),
            }
        );
    }
}
