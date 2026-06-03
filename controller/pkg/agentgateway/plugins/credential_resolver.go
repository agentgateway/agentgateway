package plugins

import "github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"

// CredentialResolverFactory builds the complete credential resolver chain for
// policy translation.
type CredentialResolverFactory func(*AgwCollections) kubeutils.CredentialResolver

// DefaultCredentialResolverFactory returns the default resolver chain.
func DefaultCredentialResolverFactory(agw *AgwCollections) kubeutils.CredentialResolver {
	if agw == nil {
		return nil
	}
	return kubeutils.NewSecretCredentialResolver(agw.Secrets)
}
