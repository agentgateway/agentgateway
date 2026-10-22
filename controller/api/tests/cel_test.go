package tests

import (
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"sigs.k8s.io/yaml"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

// Split where the '---' appears at the very beginning of a line. This will avoid
// accidentally splitting in cases where yaml resources contain nested yaml (which
// is indented).
var splitRegex = regexp.MustCompile(`(^|\n)---`)

// SplitString splits the given yaml doc if it's multipart document.
func splitString(yamlText string) []string {
	out := make([]string, 0)
	parts := splitRegex.Split(yamlText, -1)
	for _, part := range parts {
		part := strings.TrimSpace(part)
		if len(part) > 0 {
			out = append(out, part)
		}
	}
	return out
}

type TestExpectation struct {
	WantErr string `json:"_err,omitempty"`
}

func TestExamples(t *testing.T) {
	v := NewAgentgatewayValidatorSkipMissing(t)
	root, err := os.OpenRoot(filepath.Join(fsutils.GetModuleRoot(), "examples"))
	if err != nil {
		t.Fatal(err)
	}
	defer root.Close()
	err = fs.WalkDir(root.FS(), ".", func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() || filepath.Ext(path) != ".yaml" {
			return err
		}
		content, err := root.ReadFile(path)
		if err != nil {
			return err
		}
		for _, item := range splitString(string(content)) {
			obj := &unstructured.Unstructured{}
			if err := yaml.Unmarshal([]byte(item), obj); err != nil || obj.GetKind() == "" {
				continue
			}
			t.Run(fmt.Sprintf("%s/%s/%s", path, obj.GetKind(), obj.GetName()), func(t *testing.T) {
				assert.NoError(t, v.ValidateCustomResource(obj))
			})
		}
		return nil
	})
	assert.NoError(t, err)
}

func TestCRDs(t *testing.T) {
	v := NewAgentgatewayValidatorStrict(t)
	base := "testdata"
	d, err := os.ReadDir(base)
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range d {
		t.Run(f.Name(), func(t *testing.T) {
			f, err := os.ReadFile(filepath.Join("testdata", f.Name()))
			if err != nil {
				t.Fatal(err)
			}
			for _, item := range splitString(string(f)) {
				obj := &unstructured.Unstructured{}
				if err := yaml.Unmarshal([]byte(item), obj); err != nil {
					t.Fatal(err)
				}
				delete(obj.Object, "_err")
				t.Run(obj.GetName(), func(t *testing.T) {
					want := TestExpectation{}
					if err := yaml.Unmarshal([]byte(item), &want); err != nil {
						t.Fatal(err)
					}
					res := v.ValidateCustomResource(obj)
					if want.WantErr == "" {
						// Want no error
						if res != nil {
							t.Fatalf("configuration was invalid: %v", res)
						}
					} else {
						if res == nil {
							t.Fatalf("wanted error like %q, got none", want.WantErr)
						}
						if !strings.Contains(res.Error(), want.WantErr) {
							t.Fatalf("wanted error like %q, got %q", want.WantErr, res)
						}
					}
				})
			}
		})
	}
}
