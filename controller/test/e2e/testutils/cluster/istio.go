//go:build e2e

package cluster

import (
	"context"
	"errors"
	"fmt"
	"log"
	"log/slog"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"

	testruntime "github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/runtime"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

const (
	// TODO(npolshak): Add support for other profiles (ambient, etc.)
	minimalProfile = "minimal"
)

func GetIstioctl(ctx context.Context) (string, error) {
	// Download istioctl binary
	istioctlBinary, err := downloadIstio(getIstioVersion())
	if err != nil {
		return "", fmt.Errorf("failed to download istio: %w", err)
	}
	slog.Info("using Istio binary", "path", istioctlBinary)

	return istioctlBinary, nil
}

func InstallMinimalIstio(
	ctx context.Context,
	istioctlBinary, kubeContext string,
) error {
	operatorFileContent := generateIstioOperatorFileContent("", minimalProfile)
	operatorFile := filepath.Join(os.TempDir(), "istio-operator.yaml")

	err := os.WriteFile(operatorFile, []byte(operatorFileContent), 0o600)
	if err != nil {
		return fmt.Errorf("failed to write operator file: %w", err)
	}

	return installIstioOperator(ctx, istioctlBinary, kubeContext, operatorFile)
}

func InstallRevisionedIstio(
	ctx context.Context,
	istioctlBinary, kubeContext, revision, profile string,
	extraArgs ...string,
) error {
	operatorFileContent := generateIstioOperatorFileContent(revision, profile)
	operatorFile := filepath.Join(os.TempDir(), "istio-operator.yaml")

	err := os.WriteFile(operatorFile, []byte(operatorFileContent), 0o600)
	if err != nil {
		return fmt.Errorf("failed to write operator file: %w", err)
	}

	return installIstioOperator(ctx, istioctlBinary, kubeContext, operatorFile, extraArgs...)
}

// TODO(npolshak): Add Istio dependency to define operator in code instead of writing file
func generateIstioOperatorFileContent(revision, profile string) string {
	// use minimal as the default profile if none is provided
	if profile == "" {
		profile = minimalProfile
	}

	baseContent := `
apiVersion: install.istio.io/v1alpha1
kind: IstioOperator
metadata:
  namespace: istio-system
spec:
  profile: %s
`
	if revision != "" {
		return fmt.Sprintf(baseContent+"  revision: %s\n", profile, revision)
	}
	return fmt.Sprintf(baseContent, profile)
}

func installIstioOperator(
	ctx context.Context,
	istioctlBinary, kubeContext, operatorFile string,
	extraArgs ...string,
) error {
	if testutils.ShouldSkipIstioInstall() {
		return nil
	}

	//  istioctl install -y --context <kube-context> -f <operator-file>
	args := append([]string{
		"install", "-y", "--context", kubeContext, "-f", operatorFile,
	}, extraArgs...)
	cmd := exec.Command(istioctlBinary, args...)

	if out, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("istioctl install failed: %w\noutput:\n%s", err, string(out))
	}

	return ctx.Err()
}

func getIstioVersion() string {
	if version := os.Getenv(testruntime.IstioVersionEnv); version != "" {
		return version
	} else {
		// Fail loudly if ISTIO_VERSION is not set
		panic(fmt.Sprintf("%s environment variable must be specified to run", testruntime.IstioVersionEnv))
	}
}

// Download istioctl binary from istio.io/downloadIstio and returns the path to the binary
func downloadIstio(version string) (string, error) {
	if version == "" {
		slog.Info("ISTIO_VERSION not specified, using istioctl from PATH")
		binaryPath, err := exec.LookPath("istioctl")
		if err != nil {
			return "", errors.New("ISTIO_VERSION environment variable must be specified or istioctl must be installed")
		}

		slog.Info("using istioctl", "path", binaryPath)

		return binaryPath, nil
	}
	installLocation := filepath.Join(testutils.GitRootDirectory(), ".bin")
	binaryDir := filepath.Join(installLocation, fmt.Sprintf("istio-%s", version), "bin")
	binaryLocation := filepath.Join(binaryDir, "istioctl")

	fileInfo, _ := os.Stat(binaryLocation)
	if fileInfo != nil {
		return binaryLocation, nil
	}
	if err := os.MkdirAll(binaryDir, 0o755); err != nil {
		return "", fmt.Errorf("create directory: %w", err)
	}

	if istioctlDownloadFrom := os.Getenv("ISTIOCTL_DOWNLOAD_FROM"); istioctlDownloadFrom != "" {
		osName := "linux"
		if runtime.GOOS == "darwin" {
			osName = "osx"
		}

		arch := runtime.GOARCH
		archModifier := fmt.Sprintf("-%s", arch)

		if osName == "osx" && arch != "arm64" {
			archModifier = ""
		}

		url := fmt.Sprintf("%s/%s/istioctl-%s-%s%s.tar.gz", istioctlDownloadFrom, version, version, osName, archModifier)

		// Use curl and tar to download and extract the file
		cmd := exec.Command("sh", "-c", fmt.Sprintf("curl -sSL %s | tar -xz -C %s", url, binaryDir)) //nolint:gosec // G204: controlled download command for istioctl binary in tests
		if err := cmd.Run(); err != nil {
			return "", fmt.Errorf("download and extract istioctl, cmd: %s: %w", cmd.Args, err)
		}
		// Change permissions
		if err := os.Chmod(binaryLocation, 0o755); err != nil {
			return "", fmt.Errorf("change permissions: %w", err)
		}
		return binaryLocation, nil
	}

	req, err := http.NewRequest(http.MethodGet, "https://istio.io/downloadIstio", nil)
	if err != nil {
		return "", err
	}

	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return "", err
	}
	defer res.Body.Close()

	cmd := exec.Command("sh", "-")

	cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", testruntime.IstioVersionEnv, version))
	cmd.Dir = installLocation

	cmd.Stdin = res.Body
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr

	if err = cmd.Run(); err != nil {
		return "", err
	}

	return binaryLocation, err
}

func UninstallIstio(istioctlBinary, kubeContext string) error {
	// sh -c yes | istioctl uninstall —purge —context <kube-context>
	cmd := exec.Command("sh", "-c", fmt.Sprintf("yes | %s uninstall --purge --context '%s'", istioctlBinary, kubeContext)) //nolint:gosec // G204: controlled istioctl uninstall command in tests
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("istioctl uninstall failed: %w", err)
	}
	return nil
}

// CreateIstioBugReport generates an istioctl bug report and moves it to the _output directory
func CreateIstioBugReport(ctx context.Context, istioctlBinary, kubeContext, artifactOutputDir string) {
	// Generate istioctl bug report
	if istioctlBinary == "" {
		log.Fatal("istioctl binary not set. Cannot generate istioctl bug report.")
	}

	bugReportCmd := exec.Command(istioctlBinary, "bug-report", "--full-secrets", "--context", kubeContext)
	bugReportErr := bugReportCmd.Run()
	if bugReportErr != nil {
		fmt.Println("Error generating bug report:", bugReportErr)
	}
	mvCmd := exec.Command("mv", "bug-report.tar.gz", artifactOutputDir)
	mvErr := mvCmd.Run()
	if mvErr != nil {
		fmt.Println("Error moving bug report file:", mvErr)
	}
}
