use super::*;

/// Work a run did before anything went wrong: a call, a tool, its result.
const WORK: &str = r#"{"type":"assistant.turn_start","data":{"turnId":"28"}}
{"type":"model.call_start","data":{"turnId":"28","model":"gpt-6.1-sol"}}
{"type":"model.call_finished","data":{"turnId":"28","outcome":"success","dispatchDurationMs":4210}}
{"type":"assistant.message","data":{"messageId":"m28","content":"Running the verification matrix."}}
{"type":"tool.execution_start","data":{"toolCallId":"t28","toolName":"powershell","arguments":{"command":"./verify.ps1 -All"}}}
{"type":"model.call_final_result","data":{"model":"gpt-6.1-sol","isByok":false,"result":"success"}}
{"type":"tool.execution_complete","data":{"toolCallId":"t28","success":true,"result":{"content":"49/49 checks passed"}}}
{"type":"assistant.turn_end","data":{"turnId":"28"}}
"#;

/// The failure as the report of it quoted it: the call, its rejection, the session's error, the end.
const REJECTED: &str = r#"{"type":"model.call_start","data":{"turnId":"29","model":"gpt-6.1-sol","previousResponseId":"Q995"}}
{"type":"model.call_finished","data":{"turnId":"29","outcome":"error","dispatchDurationMs":606}}
{"type":"model.call_final_result","data":{"model":"gpt-6.1-sol","isByok":false,"result":"http_400"}}
{"type":"session.error","data":{"errorType":"query","message":"Execution failed: 400 internal server error (Request ID: 00000-7a12)","statusCode":400,"providerCallId":"00000-7a12","serviceRequestId":"9e19"}}
{"type":"result","sessionId":"753e","exitCode":1,"usage":{"premiumRequests":3}}
"#;

/// The same failure as a real transcript has it: the call's own failure record, the turn's end,
/// and the bookkeeping Copilot CLI writes after the error and before it exits.
const REJECTED_IN_FULL: &str = r#"{"type":"assistant.turn_start","data":{"turnId":"29"}}
{"type":"model.call_start","data":{"turnId":"29","model":"gpt-6.1-sol"}}
{"type":"model.call_failure","data":{"errorType":"websocket_error","statusCode":400,"model":"gpt-6.1-sol","providerCallId":"00000-e82f","serviceRequestId":"8b33"}}
{"type":"model.call_finished","data":{"turnId":"29","outcome":"error"}}
{"type":"model.call_final_result","data":{"model":"gpt-6.1-sol","isByok":false,"result":"http_400"}}
{"type":"assistant.turn_end","data":{"turnId":"29"}}
{"type":"session.error","data":{"errorType":"query","statusCode":400,"message":"Execution failed: 400 internal server error (Request ID: 00000-e82f)","providerCallId":"00000-e82f","serviceRequestId":"8b33"}}
{"type":"session.usage_checkpoint","data":{"totalNanoAiu":1510581560000,"totalPremiumRequests":15}}
{"type":"assistant.idle","data":{}}
{"type":"session.background_tasks_changed","data":{}}
{"type":"session.background_tasks_changed","data":{}}
{"type":"result","sessionId":"753e","exitCode":1}
"#;

fn copilot(text: &str) -> Option<ProviderFailure> {
    Reading::Copilot.classify(text)
}

#[test]
fn a_rejected_call_that_ended_the_run_is_the_model_services_failure() {
    let failure = copilot(&format!("{WORK}{REJECTED}")).expect("a provider failure");

    assert_eq!(failure.result.as_deref(), Some("http_400"));
    assert_eq!(failure.request_id.as_deref(), Some("00000-7a12"));
    assert_eq!(
        failure.message.as_deref(),
        Some("Execution failed: 400 internal server error (Request ID: 00000-7a12)")
    );
}

#[test]
fn bookkeeping_after_the_error_does_not_hide_it() {
    let failure = copilot(&format!("{WORK}{REJECTED_IN_FULL}")).expect("a provider failure");

    assert_eq!(failure.result.as_deref(), Some("http_400"));
    assert_eq!(failure.request_id.as_deref(), Some("00000-e82f"));
}

#[test]
fn either_half_of_the_failure_is_enough() {
    let final_result = r#"{"type":"model.call_final_result","data":{"result":"http_503"}}"#;
    let session = r#"{"type":"session.error","data":{"errorType":"query","message":"Execution failed: 503"}}"#;

    assert_eq!(
        copilot(&format!("{WORK}{final_result}\n"))
            .and_then(|failure| failure.result)
            .as_deref(),
        Some("http_503")
    );
    assert_eq!(
        copilot(&format!("{WORK}{session}\n"))
            .and_then(|failure| failure.message)
            .as_deref(),
        Some("Execution failed: 503")
    );
}

#[test]
fn a_failure_the_run_carried_on_from_did_not_end_it() {
    // What one real run did: the call was rejected, a background shell kept printing, a
    // notification woke the agent, and it went on to finish its work before exiting 1.
    let carried_on = format!(
        r#"{WORK}{REJECTED_IN_FULL}{{"type":"tool.shell_output","data":{{"output":"lint-01 : exit 0"}}}}
{{"type":"system.notification","data":{{"message":"background task finished"}}}}
{{"type":"assistant.message","data":{{"content":"The lint run finished."}}}}
{{"type":"tool.execution_start","data":{{"toolCallId":"t30","toolName":"read_powershell"}}}}
{{"type":"model.call_final_result","data":{{"result":"success"}}}}
{{"type":"tool.execution_complete","data":{{"toolCallId":"t30","success":true}}}}
{{"type":"result","exitCode":1}}
"#
    );

    assert_eq!(copilot(&carried_on), None);
}

#[test]
fn a_sub_agents_failure_the_session_recovered_from_is_not_the_runs() {
    // Copilot CLI reports a sub-agent's exhausted retries with the session's own error event; the
    // parent's model then carried on.
    let recovered = format!(
        r#"{WORK}{{"type":"model.call_final_result","data":{{"model":"gpt-6.1-sol","result":"transport_error"}}}}
{{"type":"session.error","data":{{"errorType":"query","message":"Execution failed: Failed to get response from the AI model; retried 5 times"}}}}
{{"type":"subagent.failed","data":{{}}}}
{{"type":"tool.execution_complete","data":{{"toolCallId":"task1","success":false}}}}
{{"type":"model.call_start","data":{{"model":"claude-opus-5.5"}}}}
{{"type":"model.call_final_result","data":{{"model":"claude-opus-5.5","result":"success"}}}}
{{"type":"assistant.turn_end","data":{{}}}}
{{"type":"result","exitCode":1}}
"#
    );

    assert_eq!(copilot(&recovered), None);
}

#[test]
fn an_error_quoted_in_a_tool_result_or_a_message_is_not_one() {
    // An agent reading a log of an earlier failure, and saying what it found. Both carry the
    // failure's own JSON as text; neither is the CLI reporting one.
    let quoted = r#"{\"type\":\"session.error\",\"data\":{\"errorType\":\"query\",\"message\":\"Execution failed: 400\"}}"#;
    let final_quoted =
        r#"{\"type\":\"model.call_final_result\",\"data\":{\"result\":\"http_400\"}}"#;
    let reading_a_log = format!(
        r#"{WORK}{{"type":"tool.execution_complete","data":{{"toolCallId":"t29","success":true,"result":{{"content":"{final_quoted}\n{quoted}"}}}}}}
{{"type":"assistant.message","data":{{"content":"The earlier run died on {quoted}"}}}}
{{"type":"result","exitCode":1}}
"#
    );

    assert_eq!(copilot(&reading_a_log), None);
}

#[test]
fn a_run_that_failed_its_own_work_is_not_the_services_failure() {
    let tests_failed = format!(
        r#"{WORK}{{"type":"tool.execution_start","data":{{"toolCallId":"t29","toolName":"powershell"}}}}
{{"type":"model.call_final_result","data":{{"result":"success"}}}}
{{"type":"tool.execution_complete","data":{{"toolCallId":"t29","success":false,"error":{{"message":"3 tests failed"}}}}}}
{{"type":"result","exitCode":1}}
"#
    );
    let refused = format!(
        "{WORK}{{\"type\":\"session.error\",\"data\":{{\"errorType\":\"authentication\",\"message\":\"Not signed in\"}}}}\n"
    );

    assert_eq!(copilot(&tests_failed), None);
    assert_eq!(
        copilot(&refused),
        None,
        "only a failed query is the model service's"
    );
    assert_eq!(copilot("error: could not read the MCP config\n"), None);
    assert_eq!(copilot(""), None);
}

#[test]
fn diagnostics_on_stderr_between_the_events_are_skipped() {
    let noisy = format!("{WORK}npm WARN deprecated something\n{REJECTED}Error: exiting\n");

    assert!(copilot(&noisy).is_some());
}

#[test]
fn a_runner_that_says_not_to_read_its_output_never_has_one() {
    assert_eq!(
        Reading::Nothing.classify(&format!("{WORK}{REJECTED}")),
        None
    );
}

fn declared(text: &str) -> ProviderErrors {
    toml::from_str(text).expect("parses")
}

const CODEX_LIKE: &str = r#"
failed     = [{ type = "error", error.kind = "provider" }, { type = "turn.failed", status = 503 }]
carried_on = [{ type = "item.started" }]
request_id = "error.request_id"
"#;

#[test]
fn a_declared_failure_with_nothing_after_it_ended_the_run() {
    let errors = declared(CODEX_LIKE);
    let reading = Reading::Declared(&errors);
    let ended = r#"{"type":"item.started","item":{"type":"command"}}
{"type":"error","error":{"kind":"provider","request_id":"req_42","message":"overloaded"}}
{"type":"turn.completed"}
"#;

    let failure = reading.classify(ended).expect("a provider failure");

    assert_eq!(failure.request_id.as_deref(), Some("req_42"));
    assert_eq!(
        reading.classify(r#"{"type":"turn.failed","status":503}"#),
        Some(ProviderFailure::default()),
        "a number matches a number"
    );
}

#[test]
fn a_declared_failure_the_run_carried_on_from_did_not_end_it() {
    let errors = declared(CODEX_LIKE);
    let carried_on = r#"{"type":"error","error":{"kind":"provider"}}
{"type":"item.started","item":{"type":"command"}}
"#;

    assert_eq!(Reading::Declared(&errors).classify(carried_on), None);
}

#[test]
fn a_declared_shape_matches_fields_not_text() {
    let errors = declared(CODEX_LIKE);
    let reading = Reading::Declared(&errors);

    for line in [
        // The same words, as a string.
        r#"{"type":"agent_message","text":"{\"type\":\"error\",\"error\":{\"kind\":\"provider\"}}"}"#,
        // The right type with the wrong kind, and a kind with no type.
        r#"{"type":"error","error":{"kind":"tool"}}"#,
        r#"{"error":{"kind":"provider"}}"#,
        // A string where the shape gives a number.
        r#"{"type":"turn.failed","status":"503"}"#,
        "type=error kind=provider",
    ] {
        assert_eq!(reading.classify(line), None, "{line}");
    }
}

#[test]
fn what_the_cli_said_is_said_once_with_its_request() {
    let failure = copilot(&format!("{WORK}{REJECTED}")).expect("a provider failure");

    assert_eq!(
        failure.to_string(),
        "the model service failed (http_400): Execution failed: 400 internal server error \
         (Request ID: 00000-7a12)"
    );
    assert_eq!(
        ProviderFailure {
            result: Some("transport_error".to_owned()),
            request_id: Some("req_1".to_owned()),
            message: None,
        }
        .to_string(),
        "the model service failed (transport_error) (request req_1)"
    );
}

/// A run that never started its work: Copilot CLI loaded its MCP servers, then found the model it
/// was given not available and said so in a line of plain text, as one did in a short outage on
/// 26 October 2026.
const MODEL_GONE: &str = r#"{"type":"session.mcp_server_starting","data":{"serverName":"layover"}}
{"type":"session.mcp_server_tools_loaded","data":{"serverName":"layover","toolCount":10}}
Error: Model "gpt-6.1-sol" from --model flag is not available.
"#;

#[test]
fn a_model_not_available_at_start_up_is_the_model_services_failure() {
    let failure = copilot(MODEL_GONE).expect("a provider failure");

    assert_eq!(
        failure.message.as_deref(),
        Some(r#"Model "gpt-6.1-sol" from --model flag is not available."#),
        "the CLI's own words, so a person can tell an outage from a typo"
    );
    assert_eq!(failure.result, None);
    assert_eq!(failure.request_id, None);
    assert_eq!(
        failure.to_string(),
        r#"the model service failed: Model "gpt-6.1-sol" from --model flag is not available."#
    );
}

#[test]
fn every_known_wording_of_it_is_read() {
    for (before, after) in MODEL_UNAVAILABLE {
        let line = format!("{before}claude-opus-5.5{after}\r\n");
        assert!(copilot(&line).is_some(), "{line}");
    }
}

#[test]
fn the_start_up_line_quoted_inside_an_event_is_not_one() {
    // An agent reading the log of an earlier run, and saying what it found: the same words, as a
    // string inside an event, which is never compared against them.
    let line = r#"Error: Model \"gpt-6.1-sol\" from --model flag is not available."#;
    let quoting = format!(
        r#"{WORK}{{"type":"tool.execution_complete","data":{{"toolCallId":"t29","success":true,"result":{{"content":"{line}"}}}}}}
{{"type":"assistant.message","data":{{"content":"The tester died with {line}"}}}}
{{"type":"result","exitCode":1}}
"#
    );

    assert_eq!(copilot(&quoting), None);
}

#[test]
fn the_start_up_line_counts_only_whole_and_only_last() {
    let carried_on_with_a_call = format!(
        "{MODEL_GONE}{{\"type\":\"model.call_final_result\",\"data\":{{\"result\":\"success\"}}}}\n"
    );
    let carried_on_with_a_tool = format!(
        "{MODEL_GONE}{{\"type\":\"tool.execution_start\",\"data\":{{\"toolCallId\":\"t1\",\"toolName\":\"powershell\"}}}}\n"
    );
    assert_eq!(copilot(&carried_on_with_a_call), None);
    assert_eq!(copilot(&carried_on_with_a_tool), None);

    for not_whole in [
        "warning: Error: Model \"gpt-6.1-sol\" from --model flag is not available.",
        "Error: Model \"gpt-6.1-sol\" from --model flag is not available. Retrying.",
        "Error: Model \"\" from --model flag is not available.",
        "Error: Model \"gpt-6.1-sol\" from --model flag is not supported.",
    ] {
        assert_eq!(copilot(not_whole), None, "{not_whole}");
    }
}
