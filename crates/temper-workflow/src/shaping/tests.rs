use super::*;

fn create(home: Home) -> CreateInput {
    CreateInput {
        title: "T".into(),
        doc_type: "research".into(),
        home,
        content: None,
        sources: Vec::new(),
        managed_meta: None,
        open_meta: None,
        goal: None,
        origin_uri: None,
        idempotency_key: None,
        act: ActInput::default(),
    }
}

#[test]
fn a_create_with_no_body_sends_no_body() {
    let p = shape_create(create(Home::Context("@me/ctx".into()))).unwrap();
    assert_eq!(p.content, "");
    assert!(p.content_hash.is_none() && p.chunks_packed.is_none());
}

#[test]
fn an_absent_origin_is_empty() {
    let p = shape_create(create(Home::Context("@me/ctx".into()))).unwrap();
    assert_eq!(p.origin_uri, "");
}

#[test]
fn an_absent_idempotency_key_is_minted_and_a_given_one_is_kept() {
    let minted = shape_create(create(Home::Context("@me/ctx".into()))).unwrap();
    assert!(minted.idempotency_key.is_some());

    let key = Uuid::now_v7();
    let mut input = create(Home::Context("@me/ctx".into()));
    input.idempotency_key = Some(key);
    assert_eq!(shape_create(input).unwrap().idempotency_key, Some(key));
}

#[test]
fn a_cogmap_home_sends_an_empty_context_ref() {
    let map = Uuid::now_v7();
    let p = shape_create(create(Home::Cogmap(map))).unwrap();
    assert_eq!(p.context_ref, "");
    assert_eq!(p.home_cogmap_id, Some(map));
}

#[test]
fn a_context_home_sends_the_ref_as_written() {
    let p = shape_create(create(Home::Context("+team/notes".into()))).unwrap();
    assert_eq!(p.context_ref, "+team/notes");
    assert!(p.home_cogmap_id.is_none());
}

#[test]
fn an_empty_managed_tier_is_not_sent_on_create() {
    let mut input = create(Home::Context("@me/ctx".into()));
    input.managed_meta = Some(ManagedMeta::default());
    assert!(shape_create(input).unwrap().managed_meta.is_none());
}

#[test]
fn a_managed_tier_is_sent_on_create() {
    let mut input = create(Home::Context("@me/ctx".into()));
    input.managed_meta = Some(ManagedMeta {
        mode: Some("plan".into()),
        ..ManagedMeta::default()
    });
    let mm = shape_create(input)
        .unwrap()
        .managed_meta
        .expect("managed_meta");
    assert_eq!(mm["temper-mode"], "plan");
}

#[test]
fn a_segmented_begin_is_a_create_with_its_marker() {
    let seg = SegmentedBegin {
        total_blocks_hint: Some(3),
        block_budget: 1024,
        source_hash: Some("h".into()),
    };
    let mut input = create(Home::Context("@me/ctx".into()));
    input.content = Some("segment zero".into());
    let p = shape_segmented_begin(input, seg).unwrap();
    assert_eq!(p.content, "segment zero");
    assert_eq!(p.segmented.as_ref().map(|s| s.block_budget), Some(1024));
}

#[test]
fn an_update_with_empty_content_sends_no_revise() {
    let req = shape_update(UpdateInput {
        content: Some(String::new()),
        ..UpdateInput::default()
    })
    .unwrap();
    assert!(req.content.is_none());
}

#[test]
fn an_update_with_an_empty_managed_tier_sends_none() {
    let req = shape_update(UpdateInput {
        managed_meta: Some(ManagedMeta::default()),
        ..UpdateInput::default()
    })
    .unwrap();
    assert!(req.managed_meta.is_none());
}

#[test]
fn a_goal_change_is_the_wire_tri_state() {
    let id = Uuid::now_v7();
    let set = shape_update(UpdateInput {
        goal: Some(GoalChange::Set(id)),
        ..UpdateInput::default()
    })
    .unwrap();
    assert_eq!((set.goal, set.clear_goal), (Some(id), None));

    let clear = shape_update(UpdateInput {
        goal: Some(GoalChange::Clear),
        ..UpdateInput::default()
    })
    .unwrap();
    assert_eq!((clear.goal, clear.clear_goal), (None, Some(true)));

    let none = shape_update(UpdateInput::default()).unwrap();
    assert_eq!((none.goal, none.clear_goal), (None, None));
}

#[test]
fn an_update_carries_the_move_fields_as_written() {
    let req = shape_update(UpdateInput {
        context_to: Some("@me/elsewhere".into()),
        type_to: Some("field-note".into()),
        ..UpdateInput::default()
    })
    .unwrap();
    assert_eq!(req.context_to.as_deref(), Some("@me/elsewhere"));
    assert_eq!(req.type_to.as_deref(), Some("field-note"));
}

#[test]
fn an_annotate_carries_its_sources_and_block() {
    let block = Uuid::now_v7();
    let req = shape_annotate(AnnotateInput {
        sources: vec![ProvenanceSource::Resource(Uuid::now_v7())],
        content_block: Some(block),
        act: ActInput::default(),
    });
    assert_eq!(req.sources.len(), 1);
    assert_eq!(req.content_block, Some(block));
}
