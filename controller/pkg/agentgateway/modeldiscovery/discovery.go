package modeldiscovery

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"slices"
	"strings"
	"sync"
	"time"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

const (
	EnabledAnnotation    = "agentgateway.dev/model-discovery"
	PathAnnotation       = "agentgateway.dev/model-discovery-path"
	IntervalAnnotation   = "agentgateway.dev/model-discovery-interval"
	StaleAfterAnnotation = "agentgateway.dev/model-discovery-stale-after"

	defaultPath           = "/v1/models"
	defaultInterval       = 30 * time.Second
	defaultStaleAfter     = 5 * time.Minute
	defaultScanInterval   = time.Second
	defaultRequestTimeout = 5 * time.Second
	maxResponseBytes      = 1 << 20
	maxModelsPerResponse  = 10_000
	maxModelIDLength      = 1_024
	maxConcurrentPolls    = 16
	minPollInterval       = 5 * time.Second
)

var logger = logging.New("agentgateway/model-discovery")

// Model is a normalized model discovered for one AgentgatewayModel anchor.
type Model struct {
	Owner   types.NamespacedName
	ID      string
	Created uint64
}

func (m Model) ResourceName() string {
	digest := sha256.Sum256([]byte(m.ID))
	return m.Owner.String() + "/" + hex.EncodeToString(digest[:8])
}

func (m Model) Equals(other Model) bool {
	return m.Owner == other.Owner &&
		m.ID == other.ID &&
		m.Created == other.Created
}

// AnchorConfig is the validated discovery configuration for an
// AgentgatewayModel.
type AnchorConfig struct {
	Owner      types.NamespacedName
	Pool       types.NamespacedName
	Path       string
	Interval   time.Duration
	StaleAfter time.Duration
}

// ParseAnchor validates the provisional annotation-based discovery contract.
func ParseAnchor(model *agentgateway.AgentgatewayModel) (AnchorConfig, bool, error) {
	if model.Annotations[EnabledAnnotation] != "enabled" {
		return AnchorConfig{}, false, nil
	}
	cfg := AnchorConfig{
		Owner: types.NamespacedName{
			Namespace: model.Namespace,
			Name:      model.Name,
		},
		Path:       defaultPath,
		Interval:   defaultInterval,
		StaleAfter: defaultStaleAfter,
	}
	if model.Spec.Match != nil {
		return AnchorConfig{}, true, errors.New("model discovery requires spec.match to be omitted")
	}
	if model.Spec.Provider == nil || *model.Spec.Provider != agentgateway.ModelProviderCustom {
		return AnchorConfig{}, true, errors.New("model discovery requires provider Custom")
	}
	if model.Spec.Custom == nil || model.Spec.Custom.BackendRef == nil {
		return AnchorConfig{}, true, errors.New("model discovery requires custom.backendRef")
	}
	ref := model.Spec.Custom.BackendRef
	if ref.Group == nil || *ref.Group != "inference.networking.k8s.io" ||
		ref.Kind == nil || *ref.Kind != "InferencePool" {
		return AnchorConfig{}, true, errors.New("model discovery custom.backendRef must target an InferencePool")
	}
	if ref.Port != nil {
		return AnchorConfig{}, true, errors.New("model discovery does not support a backendRef port")
	}
	cfg.Pool = types.NamespacedName{Namespace: model.Namespace, Name: ref.Name}

	if raw := model.Annotations[PathAnnotation]; raw != "" {
		parsed, err := url.ParseRequestURI(raw)
		if err != nil || !strings.HasPrefix(raw, "/") || parsed.IsAbs() || parsed.Host != "" ||
			parsed.RawQuery != "" || parsed.Fragment != "" {
			return AnchorConfig{}, true, fmt.Errorf("%s must be an absolute path", PathAnnotation)
		}
		cfg.Path = raw
	}
	if raw := model.Annotations[IntervalAnnotation]; raw != "" {
		interval, err := time.ParseDuration(raw)
		if err != nil || interval < minPollInterval {
			return AnchorConfig{}, true, fmt.Errorf("%s must be at least %s", IntervalAnnotation, minPollInterval)
		}
		cfg.Interval = interval
	}
	if raw := model.Annotations[StaleAfterAnnotation]; raw != "" {
		staleAfter, err := time.ParseDuration(raw)
		if err != nil || staleAfter < cfg.Interval {
			return AnchorConfig{}, true, fmt.Errorf("%s must be at least the polling interval", StaleAfterAnnotation)
		}
		cfg.StaleAfter = staleAfter
	}
	return cfg, true, nil
}

type Options struct {
	HTTPClient   *http.Client
	ScanInterval time.Duration
	Now          func() time.Time
}

type anchorState struct {
	config      AnchorConfig
	nextPoll    time.Time
	lastSuccess time.Time
}

// Controller polls model inventories and publishes normalized entries into a
// reactive collection consumed by model translation.
type Controller struct {
	models krt.Collection[*agentgateway.AgentgatewayModel]
	pools  krt.Collection[*inf.InferencePool]
	pods   krt.Collection[*corev1.Pod]

	discovered krt.StaticCollection[Model]
	client     *http.Client
	now        func() time.Time
	scanEvery  time.Duration

	mu     sync.Mutex
	states map[types.NamespacedName]anchorState
}

func NewController(
	stop <-chan struct{},
	models krt.Collection[*agentgateway.AgentgatewayModel],
	pools krt.Collection[*inf.InferencePool],
	pods krt.Collection[*corev1.Pod],
	opts Options,
	collectionOpts ...krt.CollectionOption,
) *Controller {
	client := opts.HTTPClient
	if client == nil {
		transport := http.DefaultTransport.(*http.Transport).Clone()
		transport.Proxy = nil
		client = &http.Client{
			Transport: transport,
			Timeout:   defaultRequestTimeout,
			CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
				return http.ErrUseLastResponse
			},
		}
	}
	if opts.ScanInterval <= 0 {
		opts.ScanInterval = defaultScanInterval
	}
	if opts.Now == nil {
		opts.Now = time.Now
	}
	c := &Controller{
		models:     models,
		pools:      pools,
		pods:       pods,
		discovered: krt.NewStaticCollection[Model](nil, nil, collectionOpts...),
		client:     client,
		now:        opts.Now,
		scanEvery:  opts.ScanInterval,
		states:     map[types.NamespacedName]anchorState{},
	}
	go c.run(stop)
	return c
}

func (c *Controller) Collection() krt.Collection[Model] {
	return c.discovered
}

func (c *Controller) run(stop <-chan struct{}) {
	select {
	case <-stop:
		return
	default:
	}
	c.reconcile(context.Background())
	ticker := time.NewTicker(c.scanEvery)
	defer ticker.Stop()
	for {
		select {
		case <-stop:
			return
		case <-ticker.C:
			c.reconcile(context.Background())
		}
	}
}

func (c *Controller) reconcile(ctx context.Context) {
	c.mu.Lock()
	defer c.mu.Unlock()

	now := c.now()
	active := map[types.NamespacedName]struct{}{}
	for _, anchor := range c.models.List() {
		cfg, enabled, err := ParseAnchor(anchor)
		if !enabled {
			continue
		}
		active[cfg.Owner] = struct{}{}
		if err != nil {
			logger.Warn("invalid model discovery anchor", "anchor", cfg.Owner.String(), "error", err)
			c.removeOwner(cfg.Owner)
			continue
		}
		state, found := c.states[cfg.Owner]
		if found && state.config != cfg {
			c.removeOwner(cfg.Owner)
			state = anchorState{}
			found = false
		}
		if found && now.Before(state.nextPoll) {
			continue
		}
		state.config = cfg
		state.nextPoll = now.Add(cfg.Interval)

		pool := findPool(c.pools.List(), cfg.Pool)
		endpoints := readyEndpoints(c.pods.List(), pool)
		models, successful := c.pollEndpoints(ctx, cfg, endpoints, anchor.CreationTimestamp.Unix())
		if successful {
			state.lastSuccess = now
			c.replaceOwner(cfg.Owner, models)
		} else if state.lastSuccess.IsZero() || now.Sub(state.lastSuccess) >= cfg.StaleAfter {
			c.discovered.DeleteObjects(func(model Model) bool {
				return model.Owner == cfg.Owner
			})
		}
		c.states[cfg.Owner] = state
	}
	for owner := range c.states {
		if _, found := active[owner]; !found {
			c.removeOwner(owner)
		}
	}
}

func (c *Controller) removeOwner(owner types.NamespacedName) {
	delete(c.states, owner)
	c.discovered.DeleteObjects(func(model Model) bool {
		return model.Owner == owner
	})
}

func (c *Controller) replaceOwner(owner types.NamespacedName, models []Model) {
	incoming := make(map[string]Model, len(models))
	for _, model := range models {
		incoming[model.ResourceName()] = model
		c.discovered.ConditionalUpdateObject(model)
	}
	c.discovered.DeleteObjects(func(model Model) bool {
		if model.Owner != owner {
			return false
		}
		_, found := incoming[model.ResourceName()]
		return !found
	})
}

func findPool(pools []*inf.InferencePool, key types.NamespacedName) *inf.InferencePool {
	for _, pool := range pools {
		if pool.Namespace == key.Namespace && pool.Name == key.Name {
			return pool
		}
	}
	return nil
}

type endpoint struct {
	address string
	port    int32
}

func readyEndpoints(pods []*corev1.Pod, pool *inf.InferencePool) []endpoint {
	if pool == nil || len(pool.Spec.TargetPorts) == 0 {
		return nil
	}
	matchLabels := make(labels.Set, len(pool.Spec.Selector.MatchLabels))
	for key, value := range pool.Spec.Selector.MatchLabels {
		matchLabels[string(key)] = string(value)
	}
	selector := labels.SelectorFromSet(matchLabels)
	port := int32(pool.Spec.TargetPorts[0].Number)
	var out []endpoint
	for _, pod := range pods {
		if pod.Namespace != pool.Namespace || pod.Status.Phase != corev1.PodRunning ||
			pod.Status.PodIP == "" || !selector.Matches(labels.Set(pod.Labels)) || !podReady(pod) {
			continue
		}
		out = append(out, endpoint{address: pod.Status.PodIP, port: port})
	}
	slices.SortFunc(out, func(a, b endpoint) int {
		if cmp := strings.Compare(a.address, b.address); cmp != 0 {
			return cmp
		}
		return int(a.port - b.port)
	})
	return out
}

func podReady(pod *corev1.Pod) bool {
	for _, condition := range pod.Status.Conditions {
		if condition.Type == corev1.PodReady {
			return condition.Status == corev1.ConditionTrue
		}
	}
	return false
}

type pollResult struct {
	models []responseModel
	err    error
}

func (c *Controller) pollEndpoints(
	ctx context.Context,
	cfg AnchorConfig,
	endpoints []endpoint,
	anchorCreated int64,
) ([]Model, bool) {
	if len(endpoints) == 0 {
		return nil, false
	}
	results := make(chan pollResult, len(endpoints))
	jobs := make(chan endpoint)
	workers := min(len(endpoints), maxConcurrentPolls)
	for range workers {
		go func() {
			for target := range jobs {
				models, err := c.pollEndpoint(ctx, target, cfg.Path)
				results <- pollResult{models: models, err: err}
			}
		}()
	}
	go func() {
		defer close(jobs)
		for _, target := range endpoints {
			jobs <- target
		}
	}()

	union := map[string]Model{}
	successful := false
	for range endpoints {
		result := <-results
		if result.err != nil {
			continue
		}
		successful = true
		for _, discovered := range result.models {
			created := discovered.Created
			if created == 0 && anchorCreated > 0 {
				created = uint64(anchorCreated)
			}
			current, found := union[discovered.ID]
			if !found || current.Created == 0 || created != 0 && created < current.Created {
				union[discovered.ID] = Model{Owner: cfg.Owner, ID: discovered.ID, Created: created}
			}
		}
	}
	models := make([]Model, 0, len(union))
	for _, model := range union {
		models = append(models, model)
	}
	slices.SortFunc(models, func(a, b Model) int {
		return strings.Compare(a.ID, b.ID)
	})
	return models, successful
}

type modelListResponse struct {
	Data []responseModel `json:"data"`
}

type responseModel struct {
	ID      string `json:"id"`
	Created uint64 `json:"created"`
}

func (c *Controller) pollEndpoint(ctx context.Context, target endpoint, path string) ([]responseModel, error) {
	requestURL := (&url.URL{
		Scheme: "http",
		Host:   net.JoinHostPort(target.address, fmt.Sprint(target.port)),
		Path:   path,
	}).String()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, requestURL, nil)
	if err != nil {
		return nil, err
	}
	resp, err := c.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < http.StatusOK || resp.StatusCode >= http.StatusMultipleChoices {
		return nil, fmt.Errorf("discovery endpoint returned status %d", resp.StatusCode)
	}
	return parseResponse(io.LimitReader(resp.Body, maxResponseBytes+1))
}

func parseResponse(reader io.Reader) ([]responseModel, error) {
	body, err := io.ReadAll(reader)
	if err != nil {
		return nil, err
	}
	if len(body) > maxResponseBytes {
		return nil, fmt.Errorf("model discovery response exceeds %d bytes", maxResponseBytes)
	}
	var response modelListResponse
	if err := json.Unmarshal(body, &response); err != nil {
		return nil, fmt.Errorf("parse model discovery response: %w", err)
	}
	if len(response.Data) > maxModelsPerResponse {
		return nil, fmt.Errorf("model discovery response exceeds %d entries", maxModelsPerResponse)
	}
	deduplicated := map[string]responseModel{}
	for _, model := range response.Data {
		if model.ID == "" {
			return nil, errors.New("model discovery response contains an empty model ID")
		}
		if len(model.ID) > maxModelIDLength {
			return nil, fmt.Errorf("model discovery response contains a model ID longer than %d bytes", maxModelIDLength)
		}
		current, found := deduplicated[model.ID]
		if !found || current.Created == 0 || model.Created != 0 && model.Created < current.Created {
			deduplicated[model.ID] = model
		}
	}
	out := make([]responseModel, 0, len(deduplicated))
	for _, model := range deduplicated {
		out = append(out, model)
	}
	slices.SortFunc(out, func(a, b responseModel) int {
		return strings.Compare(a.ID, b.ID)
	})
	return out, nil
}
