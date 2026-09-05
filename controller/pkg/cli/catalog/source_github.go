package catalog

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

const (
	githubSourceName = "github"
	githubCatalogURL = "https://agentgateway.dev/model-catalog"
)

func init() {
	importSources[githubSourceName] = func(ctx context.Context, opts importOptions) (*ModelCatalog, []string, error) {
		catalog, err := fetchGitHubCatalog(ctx)
		if err != nil {
			return nil, nil, err
		}
		selected := make(map[string]struct{}, len(opts.providers))
		for _, provider := range opts.providers {
			selected[provider] = struct{}{}
		}
		excluded := make(map[string]struct{}, len(opts.excludeProviders))
		for _, provider := range opts.excludeProviders {
			excluded[provider] = struct{}{}
		}
		for provider := range catalog.Providers {
			_, include := selected[provider]
			_, exclude := excluded[provider]
			if (len(selected) > 0 && !include) || exclude {
				delete(catalog.Providers, provider)
			}
		}
		return catalog, nil, nil
	}
}

func fetchGitHubCatalog(ctx context.Context) (*ModelCatalog, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, githubCatalogURL, nil)
	if err != nil {
		return nil, fmt.Errorf("build request for GitHub model catalog: %w", err)
	}
	resp, err := (&http.Client{Timeout: 30 * time.Second}).Do(req)
	if err != nil {
		return nil, fmt.Errorf("fetch GitHub model catalog: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("fetch GitHub model catalog: unexpected status %d", resp.StatusCode)
	}
	var catalog ModelCatalog
	if err := json.NewDecoder(io.LimitReader(resp.Body, 64<<20)).Decode(&catalog); err != nil {
		return nil, fmt.Errorf("decode GitHub model catalog: %w", err)
	}
	if err := catalog.Validate(); err != nil {
		return nil, fmt.Errorf("invalid GitHub model catalog: %w", err)
	}
	if catalog.Metadata != nil {
		catalog.Metadata.Source = ""
	}
	return &catalog, nil
}
