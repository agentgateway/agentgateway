package syncer

import (
	"testing"

	"google.golang.org/protobuf/types/known/structpb"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
)

func TestInferencePoolServiceExtensions(t *testing.T) {
	t.Run("single target port", func(t *testing.T) {
		extensions := inferencePoolServiceExtensions([]inf.Port{{
			Number: 8000,
		}})

		if extensions != nil {
			t.Fatalf("expected no extension for single-port inference pool, got %d", len(extensions))
		}
	})

	t.Run("multiple target ports", func(t *testing.T) {
		extensions := inferencePoolServiceExtensions([]inf.Port{
			{Number: 8000},
			{Number: 8001},
		})

		if len(extensions) != 1 {
			t.Fatalf("expected one extension, got %d", len(extensions))
		}
		if extensions[0].Name != inferencePoolServiceExtensionName {
			t.Fatalf("unexpected extension name %q", extensions[0].Name)
		}

		var config structpb.Struct
		if err := extensions[0].Config.UnmarshalTo(&config); err != nil {
			t.Fatalf("unmarshal extension config: %v", err)
		}

		value, ok := config.Fields[inferencePoolCanonicalPortStructKey]
		if !ok {
			t.Fatalf("missing %q field", inferencePoolCanonicalPortStructKey)
		}
		if got := value.GetNumberValue(); got != 8000 {
			t.Fatalf("expected canonical port 8000, got %v", got)
		}
	})
}
