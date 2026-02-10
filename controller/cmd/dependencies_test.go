package cmd_test

import (
	"bytes"
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	slices0 "slices"
	"strings"
	"testing"

	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/util/sets"

	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

// TestDependencies controls which binaries can import which packages.
// Note we also use linting (depguard), but that only handles direct dependencies, while this covers indirect ones.
func TestDependencies(t *testing.T) {
	tests := []struct {
		entrypoint string
		tag        string
		// Regex of dependencies we do not allow
		denied []string
		// Exceptions to the above. This allows a denial like `foo/` with an exception for `foo/bar/bz`
		exceptions []string
		// Things we wish were not import
		wantToDeny []string
	}{
		{
			entrypoint: "cmd/kgateway",
			tag:        "agent",
			denied: []string{
				// Deps meant only for other components; if we import them, something may be wrong
				`^testing$`,
			},
			wantToDeny: []string{
				`^github\.com/fatih/color`,
				`^helm\.sh/helm/v3`,
				`^sigs\.k8s\.io/controller-runtime/pkg/client`,
				`^github\.com/pmezard/go-difflib`,
			},
		},
	}
	allDenials := []*regexp.Regexp{}
	for _, tt := range tests {
		t.Run(tt.entrypoint, func(t *testing.T) {
			deps, err := getDependencies(filepath.Join(testutils.GitRootDirectory(), tt.entrypoint), tt.tag, false)
			assert.NoError(t, err)
			denies, err := slices.MapErr(tt.denied, regexp.Compile)
			allDenials = append(allDenials, denies...)
			assert.NoError(t, err)
			exceptions, err := slices.MapErr(tt.exceptions, regexp.Compile)
			assert.NoError(t, err)
			maybeCanMoveToDeny := map[string]*regexp.Regexp{}
			for _, wd := range tt.wantToDeny {
				maybeCanMoveToDeny[wd] = regexp.MustCompile(wd)
			}
			assert.NoError(t, err)
			unseenExceptions := sets.New[string](tt.exceptions...)
			for _, dep := range deps {
				for _, deny := range denies {
					if !deny.MatchString(dep) {
						continue
					}
					allowed := false
					for _, allow := range exceptions {
						if allow.MatchString(dep) {
							unseenExceptions.Delete(allow.String())
							allowed = true
							break
						}
					}
					if !allowed {
						t.Errorf("illegal dependency: %v", dep)
					}
				}
				for _, wantDeny := range maybeCanMoveToDeny {
					if wantDeny.MatchString(dep) {
						// We want to deny it, but we are depending on it.. cannot deny it
						delete(maybeCanMoveToDeny, wantDeny.String())
					}
				}
			}
			for us := range unseenExceptions {
				t.Errorf("exception %q was never matched, maybe redundant?", us)
			}
			for rgx := range maybeCanMoveToDeny {
				t.Errorf("we wanted to deny %q, and it was not found! move it to the denials list", rgx)
			}
			t.Logf("%d total dependencies", len(deps))
		})
	}
	t.Run("exhaustive", func(t *testing.T) {
		all, err := getDependencies(testutils.GitRootDirectory()+"/...", "integ,e2e,conformance", true)
		assert.NoError(t, err)
		for _, d := range allDenials {
			found := slices0.ContainsFunc(all, d.MatchString)
			if !found {
				t.Errorf("Had a deny rule %q, but it doesn't match *any* dependency in the repo. This is likely a bug.", d)
			}
		}
		t.Logf("%d total dependencies", len(all))
	})
}

func getDependencies(path, tag string, tests bool) ([]string, error) {
	args := []string{"list", "-mod=readonly", "-f", `{{ join .Deps "\n" }}`, "-tags=" + tag}
	if tests {
		args = append(args, "-test")
	}
	args = append(args, path)
	cmd := exec.Command("go", args...)
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()
	if err != nil {
		return nil, fmt.Errorf("%v: %v", err, stderr.String())
	}
	modules := strings.Split(stdout.String(), "\n")
	modules = slices.Sort(modules)
	modules = slices.FilterDuplicatesPresorted(modules)

	return modules, nil
}
