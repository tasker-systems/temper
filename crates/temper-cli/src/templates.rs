use askama::Template;

#[derive(Template)]
#[template(path = "task.md")]
pub struct TaskTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub slug: &'a str,
    pub context: &'a str,
    pub goal: &'a str,
    pub mode: &'a str,
    pub effort: &'a str,
    pub seq: &'a str,
    pub datetime: &'a str,
}

#[derive(Template)]
#[template(path = "session.md")]
pub struct SessionTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub date: &'a str,
}

#[derive(Template)]
#[template(path = "goal.md")]
pub struct GoalTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub slug: &'a str,
    pub context: &'a str,
    pub seq: &'a str,
    pub date: &'a str,
}

#[derive(Template)]
#[template(path = "research.md")]
pub struct ResearchTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub date: &'a str,
    pub project: &'a str,
    pub slug: &'a str,
}

#[derive(Template)]
#[template(path = "concept.md")]
pub struct ConceptTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub date: &'a str,
    pub project: &'a str,
    pub slug: &'a str,
}

#[derive(Template)]
#[template(path = "decision.md")]
pub struct DecisionTemplate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub date: &'a str,
    pub project: &'a str,
    pub slug: &'a str,
}

#[derive(Template)]
#[template(path = "skill.md")]
pub struct SkillTemplate<'a> {
    pub config_hash: &'a str,
    pub vault_path: &'a str,
    pub context_list: &'a str,
}

#[derive(Template)]
#[template(path = "command-wrapper.md")]
pub struct CommandWrapperTemplate<'a> {
    pub config_hash: &'a str,
}
