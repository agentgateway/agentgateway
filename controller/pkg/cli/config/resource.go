package config

import (
	"context"
	"fmt"
	"slices"

	"istio.io/istio/istioctl/pkg/util/handlers"
	"istio.io/istio/pkg/kube"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/cli-runtime/pkg/resource"
	openapiclient "k8s.io/client-go/openapi"
	"k8s.io/kubectl/pkg/cmd/util"
	"k8s.io/kubectl/pkg/util/openapi"
	"k8s.io/kubectl/pkg/validation"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

func resolveResourceName(ctx context.Context, kubeClient kube.CLIClient, namespace string, args []string) (string, error) {
	if len(args) == 1 {
		return args[0], nil
	}
	return inferSingleGatewayResourceName(ctx, kubeClient, namespace)
}

func inferSingleGatewayResourceName(ctx context.Context, kubeClient kube.CLIClient, namespace string) (string, error) {
	gateways, err := kubeClient.GatewayAPI().GatewayV1().Gateways(namespace).List(ctx, metav1.ListOptions{})
	if err != nil {
		return "", fmt.Errorf("failed to list Gateways in namespace %q: %w", namespace, err)
	}

	return singleGatewayResourceName(gateways.Items, namespace)
}

func singleGatewayResourceName(gateways []gwv1.Gateway, namespace string) (string, error) {
	switch len(gateways) {
	case 0:
		return "", fmt.Errorf("no Gateways found in namespace %q; pass a resource explicitly", namespace)
	case 1:
		return "gateway/" + gateways[0].Name, nil
	default:
		return "", fmt.Errorf("found %d Gateways in namespace %q; pass a resource explicitly", len(gateways), namespace)
	}
}

func resolvePodForResource(kubeClient kube.CLIClient, resourceName, namespace string) (string, string, error) {
	factory := MakeKubeFactory(kubeClient)
	pods, podNamespace, err := handlers.InferPodsFromTypedResource(resourceName, namespace, factory)
	if err != nil {
		return "", "", err
	}
	if len(pods) == 0 {
		return "", "", fmt.Errorf("no pods found for resource %q", resourceName)
	}
	slices.Sort(pods)
	return pods[0], podNamespace, nil
}
func MakeKubeFactory(k kube.CLIClient) util.Factory {
	kf := k.UtilFactory()
	return Factory{
		PartialFactory: kf,
		full:           util.NewFactory(kf),
	}
}

type Factory struct {
	kube.PartialFactory
	full util.Factory
}

func (f Factory) NewBuilder() *resource.Builder {
	return f.full.NewBuilder()
}

func (f Factory) ClientForMapping(mapping *meta.RESTMapping) (resource.RESTClient, error) {
	return f.full.ClientForMapping(mapping)
}

func (f Factory) UnstructuredClientForMapping(mapping *meta.RESTMapping) (resource.RESTClient, error) {
	return f.full.UnstructuredClientForMapping(mapping)
}

func (f Factory) Validator(validationDirective string) (validation.Schema, error) {
	return f.full.Validator(validationDirective)
}

func (f Factory) OpenAPISchema() (openapi.Resources, error) {
	return f.full.OpenAPISchema()
}

func (f Factory) OpenAPIV3Client() (openapiclient.Client, error) {
	return f.full.OpenAPIV3Client()
}

var _ util.Factory = Factory{}
