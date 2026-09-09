//go:build e2e

package e2e_test

import (
	"fmt"
	"io"
	"regexp"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
)

const (
	collectorLogTimeout = 20 * time.Second
	collectorLogPoll    = 500 * time.Millisecond
	otelTestIDHeader    = "x-otel-test-id"
)

var (
	collectorSection         = regexp.MustCompile(`(?m)^[ \t]*(?:ResourceSpans?|ScopeSpans?|Span|ResourceLogs?|ScopeLogs?|LogRecord) #\d+[ \t]*$`)
	deprecatedHTTPAttributes = [...]string{"src.addr", "http.method", "http.host", "http.path", "http.version", "http.status"}
	otelHTTPAttributes       = [...]string{
		"client.address",
		"http.request.method",
		"server.address",
		"url.path",
		"url.query",
		"url.scheme",
		"network.protocol.version",
		"http.response.status_code",
	}
)

func TestOTel(tt *testing.T) {
	t := New(tt)
	t.ApplyPersistent(otelManifest("setup.yaml"))
	t.Apply(otelManifest("route.yaml"))

	t.Run("Tracing", func(t base.Test) {
		testOTelTracing(t)
	})
	t.Run("AccessLog", func(t base.Test) {
		testOTelAccessLog(t)
	})
	t.Run("StdoutLegacy", func(t base.Test) {
		testStdoutAccessLog(
			t,
			"accesslog-stdout-legacy.yaml",
			"agw-accesslog-stdout-legacy",
			false,
		)
	})
	t.Run("StdoutOtel", func(t base.Test) {
		testStdoutAccessLog(
			t,
			"accesslog-stdout-otel.yaml",
			"agw-accesslog-stdout-otel",
			true,
		)
	})
}

func testOTelTracing(t base.Test) {
	t.Apply(otelManifest("tracing.yaml"))

	assertions.EventuallyAgwPolicyCondition(t, "agw", base.Namespace, "Accepted", metav1.ConditionTrue)

	testID := fmt.Sprintf("trace-%d", time.Now().UnixNano())
	marker := fmt.Sprintf("-> test.request.id: Str(%s)", testID)

	retry.UntilSuccessOrFail(t, func() error {
		t.Send(
			"www.example.com:8080/status/200?otel-test="+testID,
			base.ExpectOK(),
			curl.WithHeader(otelTestIDHeader, testID),
		)

		logs, err := getCollectorLogs(t)
		if err != nil {
			return fmt.Errorf("failed to get collector pod logs: %w", err)
		}
		span, ok := findCollectorBlock(
			logs,
			"Span",
			marker,
			`-> server.address: Str(www.example.com)`,
			`-> http.response.status_code: Int(200)`,
		)
		if !ok {
			return fmt.Errorf("no successful SERVER span found for test request %q", testID)
		}

		mustContain := []string{
			`-> client.address:`,
			`-> http.request.method: Str(GET)`,
			`-> server.address: Str(www.example.com)`,
			`-> server.port: Int(8080)`,
			`-> url.path: Str(/status/200)`,
			fmt.Sprintf("-> url.query: Str(otel-test=%s)", testID),
			`-> network.protocol.version: Str(1.1)`,
			`-> http.response.status_code: Int(200)`,
			`-> url.scheme: Str(http)`,
			`-> custom: Str(literal)`,
			marker,
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(span, line) {
				missing = append(missing, line)
			}
		}
		if len(missing) > 0 {
			return fmt.Errorf("missing required trace lines: %v", missing)
		}
		if err := rejectDeprecatedHTTPAttributes(span); err != nil {
			return fmt.Errorf("SERVER span: %w", err)
		}
		if !strings.Contains(logs, `-> deployment.environment.name: Str(production)`) ||
			!strings.Contains(logs, `-> service.version: Str(test)`) {
			return fmt.Errorf("missing expected trace resource attributes")
		}
		return nil
	}, retry.Timeout(collectorLogTimeout), retry.Delay(collectorLogPoll), retry.Message("should find traces in collector pod logs"))
}

func testOTelAccessLog(t base.Test) {
	t.Apply(otelManifest("accesslog-otlp.yaml"))

	assertions.EventuallyAgwPolicyCondition(t, "agw-accesslog", base.Namespace, "Accepted", metav1.ConditionTrue)
	testID := fmt.Sprintf("log-%d", time.Now().UnixNano())
	marker := fmt.Sprintf("-> test.request.id: Str(%s)", testID)

	retry.UntilSuccessOrFail(t, func() error {
		t.Send(
			"www.example.com/status/200?otel-test="+testID,
			base.ExpectOK(),
			curl.WithHeader(otelTestIDHeader, testID),
		)

		logs, err := getCollectorLogs(t)
		if err != nil {
			return fmt.Errorf("failed to get collector pod logs: %w", err)
		}
		record, ok := findCollectorBlock(
			logs,
			"LogRecord",
			marker,
			`-> http.response.status_code: Int(200)`,
		)
		if !ok {
			return fmt.Errorf("no successful OTLP LogRecord found for test request %q", testID)
		}

		mustContain := []string{
			`-> client.address:`,
			`-> http.request.method: Str(GET)`,
			`-> server.address: Str(www.example.com)`,
			`-> url.path: Str(/status/200)`,
			fmt.Sprintf("-> url.query: Str(otel-test=%s)", testID),
			`-> network.protocol.version: Str(1.1)`,
			`-> url.scheme: Str(http)`,
			`-> http.response.status_code: Int(200)`,
			marker,
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(record, line) {
				missing = append(missing, line)
			}
		}
		if len(missing) > 0 {
			return fmt.Errorf("missing required access log lines in collector output: %v", missing)
		}
		if strings.Contains(record, `-> server.port:`) {
			return fmt.Errorf("unexpected server.port without an explicit port")
		}
		if err := rejectDeprecatedHTTPAttributes(record); err != nil {
			return fmt.Errorf("OTLP LogRecord: %w", err)
		}
		return nil
	}, retry.Timeout(collectorLogTimeout), retry.Delay(collectorLogPoll), retry.Message("should find access logs in collector pod logs"))
}

func findLogLine(logs, marker string) (string, bool) {
	var found string
	for line := range strings.SplitSeq(logs, "\n") {
		if strings.Contains(line, marker) {
			found = line
		}
	}
	return found, found != ""
}

func logLineHasKey(line, key string) bool {
	pattern := `(^|[\s,{])"?` + regexp.QuoteMeta(key) + `"?\s*[:=]`
	return regexp.MustCompile(pattern).MatchString(line)
}

func getGatewayLogs(t base.Test) (string, error) {
	pods := &corev1.PodList{}
	err := t.TestInstallation.ClusterContext.ControllerClient.List(
		t.Ctx,
		pods,
		client.InNamespace(base.Namespace),
		client.MatchingLabels{
			"gateway.networking.k8s.io/gateway-name": "gateway",
		},
	)
	if err != nil {
		return "", err
	}

	var logs strings.Builder
	for i := range pods.Items {
		pod := &pods.Items[i]
		if pod.DeletionTimestamp != nil ||
			pod.Status.Phase != corev1.PodRunning ||
			!podReady(pod) {
			continue
		}

		podLogs, err := t.TestInstallation.ClusterContext.Client.PodLogs(
			t.Ctx,
			pod.Name,
			base.Namespace,
			"agentgateway",
			false,
		)
		if err != nil {
			return "", err
		}

		logs.WriteString(podLogs)
		logs.WriteByte('\n')
	}

	if logs.Len() == 0 {
		return "", fmt.Errorf("no running gateway pods found")
	}

	return logs.String(), nil
}

func testStdoutAccessLog(t base.Test, manifestName, policyName string, otelPreset bool) {
	t.Apply(otelManifest(manifestName))

	assertions.EventuallyAgwPolicyCondition(
		t,
		policyName,
		base.Namespace,
		"Accepted",
		metav1.ConditionTrue,
	)

	testID := fmt.Sprintf("stdout-%d", time.Now().UnixNano())

	retry.UntilSuccessOrFail(t, func() error {
		t.Send(
			"www.example.com/get?otel-test="+testID,
			base.ExpectOK(),
		)

		logs, err := getGatewayLogs(t)
		if err != nil {
			return err
		}

		line, ok := findLogLine(logs, testID)
		if !ok {
			return fmt.Errorf("no stdout access log found for test request %q", testID)
		}

		if otelPreset {
			for _, key := range otelHTTPAttributes {
				if !logLineHasKey(line, key) {
					return fmt.Errorf("OTel stdout log missing %q", key)
				}
			}
			if logLineHasKey(line, "server.port") {
				return fmt.Errorf("OTel stdout log contains server.port without an explicit port")
			}

			for _, key := range deprecatedHTTPAttributes {
				if logLineHasKey(line, key) {
					return fmt.Errorf("OTel stdout log contains deprecated %q", key)
				}
			}

			return nil
		}

		for _, key := range deprecatedHTTPAttributes {
			if !logLineHasKey(line, key) {
				return fmt.Errorf("legacy stdout log missing %q", key)
			}
		}

		for _, key := range otelHTTPAttributes {
			if logLineHasKey(line, key) {
				return fmt.Errorf("legacy stdout log contains OTel attribute %q", key)
			}
		}

		return nil
	}, retry.Timeout(collectorLogTimeout), retry.Delay(collectorLogPoll))
}

func findCollectorBlock(logs, kind, marker string, required ...string) (string, bool) {
	sections := collectorSection.FindAllStringIndex(logs, -1)
	for i, section := range sections {
		header := strings.TrimSpace(logs[section[0]:section[1]])
		if !strings.HasPrefix(header, kind+" #") {
			continue
		}
		end := len(logs)
		if i+1 < len(sections) {
			end = sections[i+1][0]
		}

		block := logs[section[0]:end]
		if !strings.Contains(block, marker) {
			continue
		}

		matches := true
		for _, requiredLine := range required {
			if !strings.Contains(block, requiredLine) {
				matches = false
				break
			}
		}
		if matches {
			return block, true
		}
	}
	return "", false
}

func rejectDeprecatedHTTPAttributes(block string) error {
	for _, key := range deprecatedHTTPAttributes {
		if strings.Contains(block, "-> "+key+":") {
			return fmt.Errorf("deprecated attribute %q is present", key)
		}
	}
	return nil
}

func otelManifest(name string) string {
	return manifest("otel", name)
}

func getCollectorPod(t base.Test) (string, error) {
	pods := &corev1.PodList{}
	err := t.TestInstallation.ClusterContext.ControllerClient.List(
		t.Ctx,
		pods,
		client.InNamespace(base.Namespace),
		client.MatchingLabels{"app.kubernetes.io/name": "opentelemetry-collector"},
	)
	if err != nil {
		return "", err
	}
	if len(pods.Items) == 0 {
		return "", fmt.Errorf("no collector pods found")
	}

	var newest *corev1.Pod
	for i := range pods.Items {
		pod := &pods.Items[i]
		if pod.DeletionTimestamp != nil || pod.Status.Phase != corev1.PodRunning || !podReady(pod) {
			continue
		}
		if newest == nil || pod.CreationTimestamp.After(newest.CreationTimestamp.Time) {
			newest = pod
		}
	}
	if newest == nil {
		return "", fmt.Errorf("no running collector pods found")
	}

	return newest.Name, nil
}

func getCollectorLogs(t base.Test) (string, error) {
	pod, err := getCollectorPod(t)
	if err != nil {
		return "", err
	}
	stream, err := t.TestInstallation.ClusterContext.Client.Kube().CoreV1().
		Pods(base.Namespace).
		GetLogs(pod, &corev1.PodLogOptions{}).
		Stream(t.Ctx)
	if err != nil {
		return "", err
	}
	defer stream.Close()

	logs, err := io.ReadAll(stream)
	if err != nil {
		return "", err
	}
	return string(logs), nil
}

func podReady(pod *corev1.Pod) bool {
	for _, condition := range pod.Status.Conditions {
		if condition.Type == corev1.PodReady {
			return condition.Status == corev1.ConditionTrue
		}
	}
	return false
}
