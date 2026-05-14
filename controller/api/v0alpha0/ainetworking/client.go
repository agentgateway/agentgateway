// POC: Minimal typed REST client for PayloadProcessor.
// This replaces a code-generated client for the POC.
package ainetworking

import (
	"context"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/runtime/serializer"
	"k8s.io/apimachinery/pkg/watch"
	"k8s.io/client-go/gentype"
	"k8s.io/client-go/rest"
)

// payloadProcessorScheme builds a minimal scheme just for this client (POC)
// to avoid import cycle with the main schemes package.
func payloadProcessorScheme() *runtime.Scheme {
	s := runtime.NewScheme()
	_ = AddToScheme(s)
	metav1.AddToGroupVersion(s, SchemeGroupVersion)
	return s
}

// PayloadProcessorClient provides typed access to PayloadProcessor resources (POC)
type PayloadProcessorClient struct {
	restClient     rest.Interface
	parameterCodec runtime.ParameterCodec
}

// NewPayloadProcessorClient creates a client for the ainetworking.x-k8s.io API group
func NewPayloadProcessorClient(config *rest.Config) (*PayloadProcessorClient, error) {
	s := payloadProcessorScheme()
	codecs := serializer.NewCodecFactory(s)

	cfg := rest.CopyConfig(config)
	cfg.GroupVersion = &schema.GroupVersion{Group: GroupName, Version: GroupVersion.Version}
	cfg.APIPath = "/apis"
	cfg.NegotiatedSerializer = codecs.WithoutConversion()

	client, err := rest.RESTClientFor(cfg)
	if err != nil {
		return nil, err
	}
	return &PayloadProcessorClient{
		restClient:     client,
		parameterCodec: runtime.NewParameterCodec(s),
	}, nil
}

// PayloadProcessors returns a namespaced interface for PayloadProcessor resources
func (c *PayloadProcessorClient) PayloadProcessors(namespace string) PayloadProcessorInterface {
	return &payloadProcessors{
		ClientWithList: gentype.NewClientWithList[*PayloadProcessor, *PayloadProcessorList](
			"payloadprocessors",
			c.restClient,
			c.parameterCodec,
			namespace,
			func() *PayloadProcessor { return &PayloadProcessor{} },
			func() *PayloadProcessorList { return &PayloadProcessorList{} },
		),
	}
}

// PayloadProcessorInterface provides typed CRUD operations (POC: only List/Watch used)
type PayloadProcessorInterface interface {
	List(ctx context.Context, opts metav1.ListOptions) (*PayloadProcessorList, error)
	Watch(ctx context.Context, opts metav1.ListOptions) (watch.Interface, error)
}

type payloadProcessors struct {
	*gentype.ClientWithList[*PayloadProcessor, *PayloadProcessorList]
}
