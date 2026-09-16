use super::{Form, NativeManagedFormError as Error, NativeManagedFormField as Field};
use machine_god_core::{
    MAX_SUBAGENT_NAME_BYTES, ManagedAgentMode, ManagedConfiguration, ManagedNotifications,
    ManagedPermissionMode, ManagedStopCondition, ManagedSubagentCommand,
};

fn configuration() -> ManagedConfiguration {
    ManagedConfiguration {
        name: "original name".into(),
        model: Some("provider/model".into()),
        effort: Some("high".into()),
        permission_mode: ManagedPermissionMode::Ask,
        notifications: ManagedNotifications {
            milestones: vec!["comma,name".into(), "quoted\"name".into()],
            report_interval_ms: Some(1000),
            report_duration_ms: Some(2000),
            stop_conditions: vec![
                ManagedStopCondition::Terminal,
                ManagedStopCondition::DurationElapsed,
            ],
            ..ManagedNotifications::default()
        },
    }
}

fn focus(form: &mut Form, field: Field) {
    let index = form
        .fields()
        .iter()
        .position(|candidate| *candidate == field)
        .unwrap();
    while form.view().selected != index {
        form.select(form.view().selected > index).unwrap();
    }
}

#[test]
fn create_defaults_inherit_authority_and_keep_terminal_notices_enabled() {
    let mut form = Form::create();
    assert_eq!(form.command(), Err(Error::InvalidValue(Field::Name)));
    form.replace(Field::Name, "new child").unwrap();
    let ManagedSubagentCommand::Create(command) = form.command().unwrap() else {
        panic!("create");
    };
    assert_eq!(command.mode, ManagedAgentMode::Persistent);
    assert_eq!(command.permission_mode, None);
    assert_eq!(command.model, None);
    assert_eq!(command.effort, None);
    assert_eq!(command.prompt, None);
    assert_eq!(command.notifications, ManagedNotifications::default());
}

#[test]
fn one_off_requires_a_standalone_prompt_and_retains_exact_unicode_content() {
    let mut form = Form::create();
    form.replace(Field::Name, "one-off").unwrap();
    focus(&mut form, Field::Mode);
    form.cycle().unwrap();
    assert_eq!(form.command(), Err(Error::InvalidValue(Field::Prompt)));
    let prompt = "独立した仕事\nemoji: 🦀\tend";
    form.replace(Field::Prompt, prompt).unwrap();
    let ManagedSubagentCommand::Create(command) = form.command().unwrap() else {
        panic!("create");
    };
    assert_eq!(command.mode, ManagedAgentMode::OneOff);
    assert_eq!(command.prompt.as_deref(), Some(prompt));
}

#[test]
fn field_limits_are_bytes_and_rejected_replacements_preserve_the_draft() {
    let mut form = Form::create();
    let exact = "é".repeat(MAX_SUBAGENT_NAME_BYTES / 2);
    form.replace(Field::Name, &exact).unwrap();
    assert_eq!(
        form.replace(Field::Name, &(exact.clone() + "x")),
        Err(Error::TooLarge(Field::Name))
    );
    assert_eq!(form.view().values[0], exact);
    assert_eq!(
        form.replace(Field::Permission, "yolo"),
        Err(Error::InvalidField)
    );
    assert_eq!(
        form.replace(Field::Interval, &"9".repeat(21)),
        Err(Error::TooLarge(Field::Interval))
    );
    assert_eq!(form.view().values[0], exact);
}

#[test]
fn configure_emits_only_edited_fields_and_preserves_the_pinned_id() {
    let original = configuration();
    let mut form = Form::configure("exact-child".into(), original).unwrap();
    assert_eq!(form.command(), Err(Error::NoChanges));
    assert_eq!(
        form.replace(Field::Prompt, "not configurable"),
        Err(Error::InvalidField)
    );
    form.replace(Field::Name, "renamed").unwrap();
    let ManagedSubagentCommand::Configure(command) = form.command().unwrap() else {
        panic!("configure");
    };
    assert_eq!(command.id, "exact-child");
    assert_eq!(command.name.as_deref(), Some("renamed"));
    assert_eq!(command.model, None);
    assert_eq!(command.effort, None);
    assert_eq!(command.permission_mode, None);
    assert_eq!(command.notifications, None);
    form.replace(Field::Model, "").unwrap();
    assert_eq!(form.command(), Err(Error::InvalidValue(Field::Model)));
    assert_eq!(form.view().values[0], "renamed");
}

#[test]
fn configuration_view_values_follow_the_visible_field_order() {
    let mut form = Form::configure("exact-child".into(), configuration()).unwrap();
    for field in form.fields() {
        focus(&mut form, *field);
        let view = form.view();
        let value = view.values[view.selected];
        match field {
            Field::Name => assert_eq!(value, "original name"),
            Field::Model => assert_eq!(value, "provider/model"),
            Field::Effort => assert_eq!(value, "high"),
            Field::Permission => assert_eq!(value, "ask"),
            Field::Started => assert_eq!(value, "no"),
            Field::Completed | Field::Failed | Field::Cancelled => assert_eq!(value, "yes"),
            Field::Interval => assert_eq!(value, "1000"),
            Field::Duration => assert_eq!(value, "2000"),
            Field::Milestones => assert_eq!(
                serde_json::from_str::<Vec<String>>(value).unwrap(),
                configuration().notifications.milestones
            ),
            Field::Mode | Field::Prompt => panic!("create-only field"),
        }
    }
}

#[test]
fn notification_edits_preserve_exact_names_and_clear_derived_duration_stop() {
    let mut form = Form::configure("exact-child".into(), configuration()).unwrap();
    focus(&mut form, Field::Started);
    form.cycle().unwrap();
    form.replace(Field::Duration, "").unwrap();
    let ManagedSubagentCommand::Configure(command) = form.command().unwrap() else {
        panic!("configure");
    };
    let policy = command.notifications.unwrap();
    assert!(policy.started);
    assert_eq!(policy.milestones, configuration().notifications.milestones);
    assert_eq!(policy.report_interval_ms, Some(1000));
    assert_eq!(policy.report_duration_ms, None);
    assert_eq!(policy.stop_conditions, vec![ManagedStopCondition::Terminal]);
}

#[test]
fn invalid_notification_input_retains_original_form_for_explicit_correction() {
    let mut form = Form::create();
    form.replace(Field::Name, "worker").unwrap();
    for value in [
        "0",
        "-1",
        "+1",
        "1.5",
        "18446744073709551616",
        "9223372036854775808",
    ] {
        form.replace(Field::Interval, value).unwrap();
        assert_eq!(
            form.command(),
            Err(Error::InvalidValue(Field::Interval)),
            "{value}"
        );
        assert_eq!(form.view().values[0], "worker");
    }
    form.replace(Field::Interval, "1000").unwrap();
    form.replace(Field::Duration, "2000").unwrap();
    form.replace(Field::Milestones, "[\"duplicate\",\"duplicate\"]")
        .unwrap();
    assert_eq!(form.command(), Err(Error::InvalidNotifications));
    form.replace(Field::Milestones, "[\"one\",\"two\"]")
        .unwrap();
    let ManagedSubagentCommand::Create(command) = form.command().unwrap() else {
        panic!("create");
    };
    assert!(
        command
            .notifications
            .stop_conditions
            .contains(&ManagedStopCondition::DurationElapsed)
    );
    assert_eq!(form.view().error, None);
}

#[test]
fn invalid_initial_configuration_is_rejected_before_becoming_editable() {
    assert!(Form::configure("../other".into(), configuration()).is_err());
    let mut invalid = configuration();
    invalid.name = "x".repeat(MAX_SUBAGENT_NAME_BYTES + 1);
    assert!(Form::configure("child".into(), invalid).is_err());
}

#[test]
fn debug_and_validation_errors_do_not_contain_user_content() {
    let mut form = Form::create();
    form.replace(Field::Name, "private-name").unwrap();
    form.replace(Field::Model, "private-invalid-model\n")
        .unwrap();
    form.replace(Field::Prompt, "private-prompt").unwrap();
    let error = form.command().unwrap_err();
    let debug = format!("{form:?} {:?} {error:?} {error}", form.view());
    for value in ["private-name", "private-invalid-model", "private-prompt"] {
        assert!(!debug.contains(value));
    }
}
