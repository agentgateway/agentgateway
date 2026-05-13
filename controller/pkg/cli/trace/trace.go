package trace

import (
	"bufio"
	"bytes"
	"context"
	"encoding/base64"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/gdamore/tcell/v2"
	"github.com/goccy/go-json"
	"github.com/pmezard/go-difflib/difflib"
	"github.com/rivo/tview"
	"github.com/spf13/cobra"
	"istio.io/istio/pkg/kube"
	"sigs.k8s.io/yaml"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
)

const (
	localForwardAddress = "127.0.0.1"
	localRuntimeAddress = "localhost"
	maxScannerTokenSize = 8 * 1024 * 1024
)

var (
	yamlKeyPattern = regexp.MustCompile(`^(\s*(?:-\s+)??)([^:\n]+):(.*)$`)
	numberPattern  = regexp.MustCompile(`^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$`)
)

type traceEnvelope struct {
	EventStart *uint64    `json:"eventStart"`
	EventEnd   uint64     `json:"eventEnd"`
	Severity   string     `json:"severity"`
	Scope      []string   `json:"scope,omitempty"`
	Message    traceEvent `json:"message"`
}

type traceEvent struct {
	Type            string            `json:"type"`
	Message         string            `json:"message,omitempty"`
	Expr            string            `json:"expr,omitempty"`
	RequestState    json.RawMessage   `json:"requestState,omitempty"`
	Result          json.RawMessage   `json:"result,omitempty"`
	Stage           string            `json:"stage,omitempty"`
	Phase           string            `json:"phase,omitempty"`
	SelectedRoute   json.RawMessage   `json:"selectedRoute,omitempty"`
	EvaluatedRoutes []json.RawMessage `json:"evaluatedRoutes,omitempty"`
	EffectivePolicy json.RawMessage   `json:"effectivePolicy,omitempty"`
	Kind            string            `json:"kind,omitempty"`
	Rules           []traceAuthzRule  `json:"rules,omitempty"`
	Target          string            `json:"target,omitempty"`
	BackendName     *string           `json:"backendName,omitempty"`
	BackendType     *string           `json:"backendType,omitempty"`
	Protocol        *string           `json:"protocol,omitempty"`
	Status          *uint16           `json:"status,omitempty"`
	Error           *string           `json:"error,omitempty"`
	Details         string            `json:"details,omitempty"`
}

type traceAuthzRule struct {
	Name    string `json:"name"`
	Matched bool   `json:"matched"`
	Mode    string `json:"mode"`
}

type traceRow struct {
	RawJSON          string
	Envelope         traceEnvelope
	Summary          string
	CurrentSnapshot  json.RawMessage
	PreviousSnapshot json.RawMessage
}

type detailMode string

const (
	detailSnapshot detailMode = "snapshot"
	detailDiff     detailMode = "snapshot diff"
	detailEvent    detailMode = "raw event"
)

type traceModel struct {
	app           *tview.Application
	table         *tview.Table
	details       *tview.TextView
	status        *tview.TextView
	mode          detailMode
	detailsActive bool
	statusMessage string
	rows          []traceRow
	headerRows    int
}

func run(cmd *cobra.Command, flags *traceFlags, resourceArg string, requestArgs []string) error {
	var (
		target *traceTarget
		err    error
	)
	if flags.local {
		target = resolveLocalTraceTarget()
	} else {
		target, err = resolveTraceTarget(cmd.Context(), flags.namespace, resourceArg)
		if err != nil {
			return err
		}
	}

	adminAddress, closeAdmin, err := traceAdminAddress(target, flags.proxyAdminPort)
	if err != nil {
		return err
	}
	defer closeAdmin()

	traceResp, err := openTraceStream(cmd.Context(), adminAddress)
	if err != nil {
		return err
	}
	defer traceResp.Body.Close()

	if flags.raw {
		return runRaw(cmd, target, traceResp.Body, requestArgs, flags.port)
	}
	return runTUI(cmd, target, traceResp.Body, requestArgs, flags.port)
}

type traceTarget struct {
	KubeClient   kube.CLIClient
	ResourceName string
	PodName      string
	PodNamespace string
	Local        bool
}

func resolveTraceTarget(ctx context.Context, namespaceOverride, resourceArg string) (*traceTarget, error) {
	namespace, err := kubeutil.LoadNamespace(namespaceOverride)
	if err != nil {
		return nil, err
	}

	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return nil, err
	}

	resourceArgs := []string{}
	if resourceArg != "" {
		resourceArgs = append(resourceArgs, resourceArg)
	}

	resourceName, err := kubeutil.ResolveResourceName(ctx, kubeClient, namespace, resourceArgs)
	if err != nil {
		return nil, err
	}

	podName, podNamespace, err := kubeutil.ResolvePodForResource(kubeClient, resourceName, namespace)
	if err != nil {
		return nil, err
	}

	return &traceTarget{
		KubeClient:   kubeClient,
		ResourceName: resourceName,
		PodName:      podName,
		PodNamespace: podNamespace,
	}, nil
}

func resolveLocalTraceTarget() *traceTarget {
	return &traceTarget{
		ResourceName: "localhost",
		Local:        true,
	}
}

func traceAdminAddress(target *traceTarget, adminPort int) (string, func(), error) {
	if target.Local {
		return fmt.Sprintf("%s:%d", localRuntimeAddress, adminPort), func() {}, nil
	}

	adminForwarder, err := target.KubeClient.NewPortForwarder(target.PodName, target.PodNamespace, localForwardAddress, 0, adminPort)
	if err != nil {
		return "", nil, fmt.Errorf("failed to create admin port-forward for %s/%s: %w", target.PodNamespace, target.PodName, err)
	}
	if err := adminForwarder.Start(); err != nil {
		adminForwarder.Close()
		return "", nil, fmt.Errorf("failed to start admin port-forward for %s/%s: %w", target.PodNamespace, target.PodName, err)
	}

	return adminForwarder.Address(), adminForwarder.Close, nil
}

func openTraceStream(ctx context.Context, adminAddress string) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, fmt.Sprintf("http://%s/debug/trace", adminAddress), nil)
	if err != nil {
		return nil, fmt.Errorf("failed to construct trace request: %w", err)
	}

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("failed to open trace stream: %w", err)
	}
	if resp.StatusCode != http.StatusOK {
		defer resp.Body.Close()
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("trace stream returned %s: %s", resp.Status, strings.TrimSpace(string(body)))
	}
	return resp, nil
}

func runRaw(cmd *cobra.Command, target *traceTarget, body io.ReadCloser, requestArgs []string, requestPort int) error {
	ctx, cancel := context.WithCancel(cmd.Context())
	defer cancel()

	requestErrCh := make(chan error, 1)
	if len(requestArgs) > 0 {
		go func() {
			requestErrCh <- triggerRequest(ctx, target, requestPort, requestArgs)
		}()
	}

	printErrCh := make(chan error, 1)
	go func() {
		printErrCh <- consumeTrace(body, func(raw string, _ traceEnvelope) error {
			_, err := fmt.Fprintln(cmd.OutOrStdout(), raw)
			return err
		})
	}()

	if len(requestArgs) == 0 {
		return <-printErrCh
	}

	for printErrCh != nil || requestErrCh != nil {
		select {
		case err := <-printErrCh:
			printErrCh = nil
			if err != nil {
				return err
			}
		case err := <-requestErrCh:
			requestErrCh = nil
			if err != nil {
				cancel()
				return err
			}
		}
	}

	return nil
}

func runTUI(cmd *cobra.Command, target *traceTarget, body io.ReadCloser, requestArgs []string, requestPort int) error {
	ctx, cancel := context.WithCancel(cmd.Context())
	defer cancel()

	app := tview.NewApplication()
	model := newTraceModel(app, target)

	var (
		runErr error
		errMu  sync.Mutex
	)
	setErr := func(err error) {
		errMu.Lock()
		defer errMu.Unlock()
		if runErr == nil {
			runErr = err
		}
	}

	eventCh := make(chan traceRow)
	parserErrCh := make(chan error, 1)
	go func() {
		defer close(eventCh)
		parserErrCh <- streamTraceRows(body, func(row traceRow) {
			eventCh <- row
		})
	}()

	requestErrCh := make(chan error, 1)
	if len(requestArgs) > 0 {
		go func() {
			requestErrCh <- triggerRequest(ctx, target, requestPort, requestArgs)
		}()
	}

	go func() {
		for row := range eventCh {
			rowCopy := row
			app.QueueUpdateDraw(func() {
				model.addRow(rowCopy)
			})
		}
	}()

	go func() {
		err := <-parserErrCh
		if err == nil {
			app.QueueUpdateDraw(func() {
				model.setStatus("stream complete, press q to exit")
			})
			return
		}
		setErr(err)
		app.QueueUpdateDraw(func() {
			model.setStatus(err.Error())
			app.Stop()
		})
	}()

	if len(requestArgs) > 0 {
		go func() {
			err := <-requestErrCh
			if err == nil {
				app.QueueUpdateDraw(func() {
					model.setStatus("request sent, waiting for trace to complete")
				})
				return
			}
			setErr(err)
			app.QueueUpdateDraw(func() {
				model.setStatus(err.Error())
				app.Stop()
			})
		}()
	}

	if err := app.SetRoot(model.root(), true).EnableMouse(false).Run(); err != nil {
		return err
	}
	cancel()

	errMu.Lock()
	defer errMu.Unlock()
	return runErr
}

func triggerRequest(ctx context.Context, target *traceTarget, requestPort int, requestArgs []string) error {
	if target.Local {
		curlArgs, err := buildCurlArgs(fmt.Sprintf("%s:%d", localRuntimeAddress, requestPort), requestArgs)
		if err != nil {
			return err
		}
		return runCurl(ctx, curlArgs)
	}

	forwarder, err := target.KubeClient.NewPortForwarder(target.PodName, target.PodNamespace, localForwardAddress, 0, requestPort)
	if err != nil {
		return fmt.Errorf("failed to create request port-forward for %s/%s:%d: %w", target.PodNamespace, target.PodName, requestPort, err)
	}
	defer forwarder.Close()
	if err := forwarder.Start(); err != nil {
		return fmt.Errorf("failed to start request port-forward for %s/%s:%d: %w", target.PodNamespace, target.PodName, requestPort, err)
	}

	curlArgs, err := buildCurlArgs(forwarder.Address(), requestArgs)
	if err != nil {
		return err
	}
	return runCurl(ctx, curlArgs)
}

func runCurl(ctx context.Context, curlArgs []string) error {
	curl := exec.CommandContext(ctx, "curl", curlArgs...)
	var stderr bytes.Buffer
	curl.Stderr = &stderr
	curl.Stdout = os.Stdout
	if err := curl.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg != "" {
			return fmt.Errorf("failed to execute traced request: %w: %s", err, msg)
		}
		return fmt.Errorf("failed to execute traced request: %w", err)
	}
	return nil
}

func buildCurlArgs(localAddress string, requestArgs []string) ([]string, error) {
	args := []string{"--silent", "--show-error", "--output", "/dev/null"}
	foundURL := false
	hostHeader := ""
	connectTo := ""

	for _, arg := range requestArgs {
		requestURL, err := url.Parse(arg)
		if err != nil || !isTraceRequestURL(requestURL) {
			args = append(args, arg)
			continue
		}

		foundURL = true
		if hostHeader == "" {
			hostHeader = requestURL.Host
			connectTo, err = curlConnectTo(localAddress, requestURL)
			if err != nil {
				return nil, err
			}
		} else if hostHeader != requestURL.Host {
			return nil, fmt.Errorf("all traced request URLs must use the same host, found %q and %q", hostHeader, requestURL.Host)
		}
		args = append(args, requestURL.String())
	}

	if !foundURL {
		return nil, fmt.Errorf("request args after -- must include at least one http:// or https:// URL")
	}
	if connectTo != "" {
		args = append(args, "--connect-to", connectTo)
	}
	return args, nil
}

func isTraceRequestURL(requestURL *url.URL) bool {
	return (requestURL.Scheme == "http" || requestURL.Scheme == "https") && requestURL.Host != ""
}

func curlConnectTo(localAddress string, requestURL *url.URL) (string, error) {
	localHost, localPort, err := net.SplitHostPort(localAddress)
	if err != nil {
		return "", fmt.Errorf("failed to parse local trace address %q: %w", localAddress, err)
	}
	requestHost := requestURL.Hostname()
	requestPort := requestURL.Port()
	if requestPort == "" {
		switch requestURL.Scheme {
		case "http":
			requestPort = "80"
		case "https":
			requestPort = "443"
		default:
			return "", fmt.Errorf("unsupported traced request scheme %q", requestURL.Scheme)
		}
	}
	if strings.Contains(localHost, ":") && !strings.HasPrefix(localHost, "[") {
		localHost = "[" + localHost + "]"
	}
	return fmt.Sprintf("%s:%s:%s:%s", requestHost, requestPort, localHost, localPort), nil
}

func consumeTrace(body io.Reader, onEvent func(raw string, envelope traceEnvelope) error) error {
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), maxScannerTokenSize)

	var dataLines []string
	flush := func() error {
		if len(dataLines) == 0 {
			return nil
		}
		raw := strings.Join(dataLines, "\n")
		dataLines = nil

		var envelope traceEnvelope
		if err := json.Unmarshal([]byte(raw), &envelope); err != nil {
			return fmt.Errorf("failed to decode trace event: %w", err)
		}
		return onEvent(raw, envelope)
	}

	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			if err := flush(); err != nil {
				return err
			}
			continue
		}
		if after, ok := strings.CutPrefix(line, "data: "); ok {
			dataLines = append(dataLines, after)
		}
	}
	if err := scanner.Err(); err != nil {
		return fmt.Errorf("failed to read trace stream: %w", err)
	}
	return flush()
}

func streamTraceRows(body io.Reader, onRow func(traceRow)) error {
	var currentSnapshot json.RawMessage
	var previousSnapshot json.RawMessage

	return consumeTrace(body, func(raw string, envelope traceEnvelope) error {
		row := traceRow{
			RawJSON:          raw,
			Envelope:         envelope,
			Summary:          summarizeEnvelope(envelope),
			CurrentSnapshot:  currentSnapshot,
			PreviousSnapshot: previousSnapshot,
		}

		if snapshot := eventSnapshot(envelope.Message); len(snapshot) > 0 {
			row.CurrentSnapshot = cloneRaw(snapshot)
			row.PreviousSnapshot = cloneRaw(currentSnapshot)
			previousSnapshot = cloneRaw(currentSnapshot)
			currentSnapshot = cloneRaw(snapshot)
		}

		onRow(row)
		return nil
	})
}

func eventSnapshot(event traceEvent) json.RawMessage {
	switch event.Type {
	case "requestSnapshot", "responseSnapshot", "cel":
		if len(event.RequestState) > 0 {
			return event.RequestState
		}
	}
	return nil
}

func cloneRaw(raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 {
		return nil
	}
	dup := make([]byte, len(raw))
	copy(dup, raw)
	return dup
}

func traceSeverityColor(severity string) tcell.Color {
	switch severity {
	case "warn":
		return tcell.ColorYellow
	case "success":
		return tcell.ColorGreen
	case "error":
		return tcell.ColorRed
	default:
		return tcell.ColorDefault
	}
}

func displayEventType(eventType string) string {
	switch eventType {
	case "requestStarted":
		return "Request Start"
	case "message":
		return "Message"
	case "cel":
		return "CEL"
	case "requestSnapshot":
		return "Snapshot"
	case "responseSnapshot":
		return "Snapshot"
	case "routeSelection":
		return "Route"
	case "policySelection":
		return "Policies"
	case "policy":
		return "Policy"
	case "policyEvent":
		return "Policy Event"
	case "authorizationResult":
		return "Authz"
	case "backendCallStart":
		return "Backend Start"
	case "backendCallResult":
		return "Backend Result"
	case "requestFinished":
		return "Request Done"
	default:
		return eventType
	}
}

func summarizeEvent(event traceEvent) string {
	switch event.Type {
	case "requestStarted":
		return "request started"
	case "message":
		return event.Message
	case "cel":
		return summarizeCEL(event.Expr, event.Result)
	case "requestSnapshot":
		return summarizeSnapshotStage(event.Stage)
	case "responseSnapshot":
		return summarizeSnapshotStage(event.Stage)
	case "routeSelection":
		return summarizeRouteSelection(event.SelectedRoute, len(event.EvaluatedRoutes))
	case "policySelection":
		return summarizePolicySelection(event.EffectivePolicy)
	case "policy":
		return summarizePolicy(event.Kind, event.Result)
	case "policyEvent":
		return truncate(fmt.Sprintf("%s: %s", event.Kind, event.Details), 120)
	case "authorizationResult":
		return summarizeAuthorizationResult(event.Result, event.Rules)
	case "backendCallStart":
		return strings.TrimSpace(fmt.Sprintf(
			"%s %s %s",
			event.Target,
			stringValue(event.BackendName),
			stringValue(event.Protocol),
		))
	case "backendCallResult":
		parts := []string{}
		if event.Target != "" {
			parts = append(parts, event.Target)
		}
		if event.Status != nil {
			parts = append(parts, fmt.Sprintf("status=%d", *event.Status))
		}
		if event.Error != nil && *event.Error != "" {
			parts = append(parts, "error="+*event.Error)
		}
		return truncate(strings.Join(parts, " "), 120)
	case "requestFinished":
		return "request finished"
	default:
		return truncate(compactJSON(event), 120)
	}
}

func summarizeEnvelope(envelope traceEnvelope) string {
	summary := summarizeEvent(envelope.Message)
	if len(envelope.Scope) == 0 {
		return summary
	}
	return truncate(strings.Join(envelope.Scope, " > ")+": "+summary, 120)
}

func summarizeCEL(expr string, result json.RawMessage) string {
	var payload struct {
		Error string `json:"error"`
	}
	if len(result) > 0 && json.Unmarshal(result, &payload) == nil && payload.Error != "" {
		return truncate(fmt.Sprintf("%s => error: %s", expr, payload.Error), 120)
	}
	return truncate(fmt.Sprintf("%s => %s", expr, compactJSON(result)), 120)
}

func summarizeSnapshotStage(stage string) string {
	switch stage {
	case "initial_request":
		return "Initial request snapshot"
	case "gateway_policies":
		return "Gateway policies snapshot"
	default:
		return stage
	}
}

func summarizeRouteSelection(selectedRoute json.RawMessage, evaluated int) string {
	selected := compactJSON(selectedRoute)
	if selected == "" {
		return fmt.Sprintf("no route selected (%d evaluated)", evaluated)
	}
	return fmt.Sprintf("selected %s (%d evaluated)", selected, evaluated)
}

func summarizePolicy(kind string, result json.RawMessage) string {
	var payload struct {
		Type    string `json:"type"`
		Reason  string `json:"reason"`
		Details string `json:"details"`
	}
	if len(result) > 0 && json.Unmarshal(result, &payload) == nil {
		switch payload.Type {
		case "skip":
			return truncate(fmt.Sprintf("%s skipped: %s", kind, payload.Reason), 120)
		case "apply":
			return truncate(fmt.Sprintf("%s: %s", kind, payload.Details), 120)
		}
	}
	return truncate(fmt.Sprintf("%s %s", kind, compactJSON(result)), 120)
}

func summarizePolicySelection(raw json.RawMessage) string {
	var payload map[string]json.RawMessage
	if len(raw) > 0 && json.Unmarshal(raw, &payload) == nil {
		keys := make([]string, 0, len(payload))
		for key := range payload {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		if len(keys) == 0 {
			return "effective policies: none"
		}
		return truncate("effective policies: "+strings.Join(keys, ", "), 120)
	}
	return truncate("effective="+compactJSON(raw), 120)
}

func summarizeAuthorizationResult(result json.RawMessage, rules []traceAuthzRule) string {
	outcome := strings.Trim(compactJSON(result), "\"")
	matchingAllowRules, matchingDenyRules, matchingRequireRules := 0, 0, 0
	for _, rule := range rules {
		if !rule.Matched {
			continue
		}
		switch rule.Mode {
		case "allow":
			matchingAllowRules++
		case "deny":
			matchingDenyRules++
		case "require":
			matchingRequireRules++
		}
	}
	switch outcome {
	case "allow":
		return fmt.Sprintf("allowed (%d allow, %d deny, %d require matches)", matchingAllowRules, matchingDenyRules, matchingRequireRules)
	case "deny":
		return fmt.Sprintf("denied (%d allow, %d deny, %d require matches)", matchingAllowRules, matchingDenyRules, matchingRequireRules)
	default:
		return fmt.Sprintf("%s (%d allow, %d deny, %d require matches)", outcome, matchingAllowRules, matchingDenyRules, matchingRequireRules)
	}
}

func compactJSON(value any) string {
	if value == nil {
		return ""
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return fmt.Sprintf("%v", value)
	}
	return string(raw)
}

func diffSnapshots(previous, current json.RawMessage) string {
	if len(current) == 0 {
		return "No snapshot available yet."
	}
	currentYAML := snapshotYAML(current)
	if len(previous) == 0 {
		return highlightYAML(currentYAML)
	}
	previousYAML := snapshotYAML(previous)
	if previousYAML == currentYAML {
		return highlightYAML(currentYAML)
	}

	previousLines := difflib.SplitLines(previousYAML + "\n")
	currentLines := difflib.SplitLines(currentYAML + "\n")
	matcher := difflib.NewMatcher(previousLines, currentLines)

	var rendered []string
	for _, op := range matcher.GetOpCodes() {
		switch op.Tag {
		case 'e':
			for _, line := range previousLines[op.I1:op.I2] {
				rendered = append(rendered, renderDiffLine("", line))
			}
		case 'd':
			for _, line := range previousLines[op.I1:op.I2] {
				rendered = append(rendered, renderDiffLine("-", line))
			}
		case 'i':
			for _, line := range currentLines[op.J1:op.J2] {
				rendered = append(rendered, renderDiffLine("+", line))
			}
		case 'r':
			for _, line := range previousLines[op.I1:op.I2] {
				rendered = append(rendered, renderDiffLine("-", line))
			}
			for _, line := range currentLines[op.J1:op.J2] {
				rendered = append(rendered, renderDiffLine("+", line))
			}
		default:
			return fmt.Sprintf("Failed to build diff: unexpected opcode %q", string(op.Tag))
		}
	}

	return strings.Join(trimTrailingEmptyLines(rendered), "\n")
}

func snapshotYAML(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}

	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return string(raw)
	}

	if topLevelMap, ok := value.(map[string]any); ok {
		for key, entry := range topLevelMap {
			if entry == nil {
				delete(topLevelMap, key)
			}
		}
		value = topLevelMap
	}
	normalizeTraceRequestStateBodies(value)

	rendered, err := yaml.Marshal(value)
	if err != nil {
		return string(raw)
	}
	return strings.TrimSpace(string(rendered))
}

func highlightYAML(text string) string {
	if text == "" {
		return ""
	}

	lines := strings.Split(text, "\n")
	for i, line := range lines {
		lines[i] = highlightYAMLLine(line)
	}
	return strings.Join(lines, "\n")
}

func highlightYAMLLine(line string) string {
	if line == "" {
		return ""
	}

	match := yamlKeyPattern.FindStringSubmatch(line)
	if match == nil {
		return tview.Escape(line)
	}

	prefix := tview.Escape(match[1])
	key := "[teal]" + tview.Escape(strings.TrimSpace(match[2])) + "[-]"
	rest := match[3]
	if rest == "" {
		return prefix + key + ":"
	}

	leadingWhitespace := rest[:len(rest)-len(strings.TrimLeft(rest, " "))]
	trimmed := strings.TrimLeft(rest, " ")
	return prefix + key + ":" + tview.Escape(leadingWhitespace) + colorizeScalar(trimmed)
}

func colorizeScalar(value string) string {
	color := "green"
	trimmed := strings.TrimSpace(value)
	switch {
	case trimmed == "null":
		color = "gray"
	case trimmed == "true" || trimmed == "false":
		color = "yellow"
	case numberPattern.MatchString(trimmed):
		color = "yellow"
	case strings.HasPrefix(trimmed, "[") || strings.HasPrefix(trimmed, "{"):
		color = "white"
	}
	return "[" + color + "]" + tview.Escape(value) + "[-]"
}

func renderDiffLine(prefix, line string) string {
	line = strings.TrimSuffix(line, "\n")
	if prefix == "" {
		return highlightYAMLLine(line)
	}

	color := "green"
	if prefix == "-" {
		color = "red"
	}
	if line == "" {
		return "[" + color + "]" + prefix + "[-]"
	}
	return "[" + color + "]" + prefix + " [-]" + highlightYAMLLine(line)
}

func trimTrailingEmptyLines(lines []string) []string {
	for len(lines) > 0 && lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	return lines
}

func eventYAML(raw string) string {
	if raw == "" {
		return ""
	}

	var value any
	if err := json.Unmarshal([]byte(raw), &value); err != nil {
		return raw
	}
	normalizeTraceEventRequestStateBodies(value)

	rendered, err := yaml.Marshal(value)
	if err != nil {
		return raw
	}
	return strings.TrimSpace(string(rendered))
}

func normalizeTraceEventRequestStateBodies(value any) {
	event, ok := value.(map[string]any)
	if !ok {
		return
	}
	message, ok := event["message"].(map[string]any)
	if !ok {
		return
	}
	normalizeTraceRequestStateBodies(message["requestState"])
}

func normalizeTraceRequestStateBodies(value any) {
	requestState, ok := value.(map[string]any)
	if !ok {
		return
	}
	normalizeTraceBody(requestState, "request")
	normalizeTraceBody(requestState, "response")
}

func normalizeTraceBody(requestState map[string]any, field string) {
	bodyOwner, ok := requestState[field].(map[string]any)
	if !ok {
		return
	}
	body, ok := bodyOwner["body"].(string)
	if !ok {
		return
	}
	decoded, err := base64.StdEncoding.DecodeString(body)
	if err != nil || !utf8.Valid(decoded) {
		return
	}
	decodedString := string(decoded)

	var jsonBody any
	if json.Unmarshal(decoded, &jsonBody) == nil {
		bodyOwner["body"] = jsonBody
		return
	}
	bodyOwner["body"] = decodedString
}

// nolint: unparam
func truncate(s string, limit int) string {
	if len(s) <= limit {
		return s
	}
	if limit <= 3 {
		return s[:limit]
	}
	return s[:limit-3] + "..."
}

func stringValue(v *string) string {
	if v == nil {
		return ""
	}
	return *v
}

func newTraceModel(app *tview.Application, target *traceTarget) *traceModel {
	table := tview.NewTable().
		SetBorders(false).
		SetSelectable(true, false).
		SetFixed(1, 0).
		SetEvaluateAllRows(true)
	table.SetBorder(true)
	table.SetBackgroundColor(tcell.ColorDefault)
	table.SetBorderColor(tcell.ColorDefault)
	table.SetTitleColor(tcell.ColorDefault)
	table.SetSelectedStyle(tcell.StyleDefault.Reverse(true))

	details := tview.NewTextView().
		SetDynamicColors(true).
		SetWrap(false).
		SetScrollable(true)
	details.SetBorder(true)
	details.SetBackgroundColor(tcell.ColorDefault)
	details.SetBorderColor(tcell.ColorDefault)
	details.SetTitleColor(tcell.ColorDefault)

	status := tview.NewTextView().
		SetDynamicColors(false).
		SetWrap(false)
	status.SetBorder(true)
	status.SetTitle("Help")
	status.SetBackgroundColor(tcell.ColorDefault)
	status.SetBorderColor(tcell.ColorDefault)
	status.SetTitleColor(tcell.ColorDefault)

	model := &traceModel{
		app:           app,
		table:         table,
		details:       details,
		status:        status,
		mode:          detailEvent,
		statusMessage: "waiting for trace data",
		headerRows:    1,
	}

	table.SetTitle(fmt.Sprintf("Events %s", target.ResourceName))
	model.setActivePane(false)
	model.updateDetailsTitle(nil)
	model.renderStatus()

	model.setHeader()
	model.renderDetails(-1)

	table.SetSelectionChangedFunc(func(row, _ int) {
		model.renderDetails(row)
	})
	table.SetInputCapture(model.handleInput)
	details.SetInputCapture(model.handleInput)

	return model
}

func (m *traceModel) root() tview.Primitive {
	main := tview.NewFlex().
		AddItem(m.table, 0, 3, true).
		AddItem(m.details, 0, 2, false)

	return tview.NewFlex().
		SetDirection(tview.FlexRow).
		AddItem(main, 0, 1, true).
		AddItem(m.status, 4, 0, false)
}

func (m *traceModel) setHeader() {
	headers := []string{"#", "Type", "Summary"}
	for col, header := range headers {
		m.table.SetCell(
			0,
			col,
			tview.NewTableCell(header).
				SetSelectable(false).
				SetExpansion(1).
				SetAttributes(tcell.AttrBold).
				SetBackgroundColor(tcell.ColorDefault),
		)
	}
}

func (m *traceModel) addRow(row traceRow) {
	rowIndex := len(m.rows)
	m.rows = append(m.rows, row)

	tableRow := rowIndex + m.headerRows
	textColor := traceSeverityColor(row.Envelope.Severity)
	for col, text := range []string{
		fmt.Sprintf("%d", rowIndex+1),
		//formatMicros(row.Envelope.EventEnd),
		//formatDuration(row.Envelope.EventStart, row.Envelope.EventEnd),
		displayEventType(row.Envelope.Message.Type),
		row.Summary,
	} {
		cell := tview.NewTableCell(text).
			SetExpansion(1).
			SetTextColor(textColor).
			SetBackgroundColor(tcell.ColorDefault).
			SetSelectedStyle(tcell.StyleDefault.Reverse(true))
		m.table.SetCell(tableRow, col, cell)
	}

	selectedRow, _ := m.table.GetSelection()
	shouldFollow := selectedRow == 0 || selectedRow == tableRow-1
	if shouldFollow {
		m.table.Select(tableRow, 0)
	}
	m.table.ScrollToEnd()
	m.setStatus(fmt.Sprintf("%d events", len(m.rows)))
}

func (m *traceModel) setStatus(text string) {
	m.statusMessage = text
	m.renderStatus()
}

func (m *traceModel) renderStatus() {
	activePane := "events"
	if m.detailsActive {
		activePane = "details"
	}
	legend := fmt.Sprintf("tab: switch pane (%s)   arrows: scroll selected pane   e/s/d: detail mode   q: quit", activePane)
	m.status.SetText(m.statusMessage + "\n" + legend)
}

func (m *traceModel) handleInput(event *tcell.EventKey) *tcell.EventKey {
	switch event.Key() {
	case tcell.KeyEscape:
		m.app.Stop()
		return nil
	case tcell.KeyTab:
		m.setActivePane(!m.detailsActive)
		return nil
	}
	switch event.Rune() {
	case 'q':
		m.app.Stop()
		return nil
	case 's':
		m.mode = detailSnapshot
		selectedRow, _ := m.table.GetSelection()
		m.renderDetails(selectedRow)
		return nil
	case 'd':
		m.mode = detailDiff
		selectedRow, _ := m.table.GetSelection()
		m.renderDetails(selectedRow)
		return nil
	case 'e':
		m.mode = detailEvent
		selectedRow, _ := m.table.GetSelection()
		m.renderDetails(selectedRow)
		return nil
	}
	return event
}

func (m *traceModel) setActivePane(detailsActive bool) {
	m.detailsActive = detailsActive
	if detailsActive {
		m.app.SetFocus(m.details)
		m.table.SetBorderColor(tcell.ColorDefault)
		m.details.SetBorderColor(tcell.ColorTeal)
	} else {
		m.app.SetFocus(m.table)
		m.table.SetBorderColor(tcell.ColorTeal)
		m.details.SetBorderColor(tcell.ColorDefault)
	}
	m.renderStatus()
}

func traceRowDuration(row *traceRow) string {
	if row == nil || row.Envelope.EventStart == nil || row.Envelope.EventEnd < *row.Envelope.EventStart {
		return ""
	}
	// nolint: gosec // not security sensitive cast
	return (time.Duration(row.Envelope.EventEnd-*row.Envelope.EventStart) * time.Microsecond).String()
}

func (m *traceModel) updateDetailsTitle(row *traceRow) {
	suffix := ""
	if duration := traceRowDuration(row); duration != "" {
		suffix = " " + duration
	}

	switch m.mode {
	case detailSnapshot:
		m.details.SetTitle(" Snapshot" + suffix + " ")
	case detailDiff:
		m.details.SetTitle(" Snapshot Diff" + suffix + " ")
	case detailEvent:
		m.details.SetTitle(" Raw Event" + suffix + " ")
	default:
		m.details.SetTitle(" Details" + suffix + " ")
	}
}

func (m *traceModel) renderDetails(tableRow int) {
	if tableRow < m.headerRows || tableRow >= len(m.rows)+m.headerRows {
		m.updateDetailsTitle(nil)
		switch m.mode {
		case detailEvent:
			m.details.SetText("No event available yet.")
		default:
			m.details.SetText("No snapshot available yet.")
		}
		return
	}

	row := m.rows[tableRow-m.headerRows]
	m.updateDetailsTitle(&row)
	switch m.mode {
	case detailDiff:
		m.details.SetText(diffSnapshots(row.PreviousSnapshot, row.CurrentSnapshot))
		m.details.ScrollToBeginning()
		return
	case detailEvent:
		m.details.SetText(highlightYAML(eventYAML(row.RawJSON)))
		m.details.ScrollToBeginning()
		return
	default:
		if len(row.CurrentSnapshot) == 0 {
			m.details.SetText("No snapshot available yet.")
			m.details.ScrollToBeginning()
			return
		}
		m.details.SetText(highlightYAML(snapshotYAML(row.CurrentSnapshot)))
		m.details.ScrollToBeginning()
	}
}
