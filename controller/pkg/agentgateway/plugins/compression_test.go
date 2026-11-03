package plugins

import (
	"testing"

	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestProcessCompressionPolicyRejectsUnknownAlgorithm(t *testing.T) {
	name := types.NamespacedName{Namespace: "ns", Name: "pol"}
	unknown := []agentgateway.CompressionAlgorithm{agentgateway.CompressionAlgorithmGzip, "Lz4"}

	for _, c := range []*agentgateway.Compression{
		{Response: &agentgateway.ResponseCompression{Algorithms: unknown}},
		{Request: &agentgateway.RequestDecompression{Algorithms: unknown}},
	} {
		if p, err := processCompressionPolicy(c, "base", name); err == nil || p != nil {
			t.Fatalf("expected error and no policy, got policy=%v err=%v", p, err)
		}
	}
}
