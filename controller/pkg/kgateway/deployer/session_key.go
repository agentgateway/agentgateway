package deployer

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"istio.io/istio/pkg/kube"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	typedcorev1 "k8s.io/client-go/kubernetes/typed/core/v1"
	"k8s.io/client-go/tools/record"
	utilretry "k8s.io/client-go/util/retry"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/metrics"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
)

const (
	managedSessionKeyLabel                      = "agentgateway.dev/managed"
	managedSessionKeyLabelValue                 = "session-key"
	managedSessionKeyGatewayNameAnnotation      = "agentgateway.dev/gateway-name"
	managedSessionKeyGatewayNamespaceAnnotation = "agentgateway.dev/gateway-namespace"
	managedSessionKeyGatewayUIDAnnotation       = "agentgateway.dev/gateway-uid"
	managedSessionKeyRotationAnnotation         = "agentgateway.dev/rotate-session-key"
	managedSessionKeyHandledRotationAnnotation  = "agentgateway.dev/handled-rotate-session-key"
	managedSessionKeyDataKey                    = "keyring"
	managedSessionKeyVolumeName                 = "session-key"
	managedSessionKeyMountPath                  = "/var/run/secrets/agentgateway"
	managedSessionKeyFileName                   = "session-keyring.json"
	sessionKeyEnvVar                            = "SESSION_KEY"
	sessionKeyringFileEnvVar                    = "SESSION_KEYRING_FILE"
	managedSessionKeyVersion                    = "v1"
	managedSessionKeyReasonCreated              = "SessionKeyCreated"
	managedSessionKeyReasonRepaired             = "SessionKeyRepaired"
	managedSessionKeyReasonConflict             = "SessionKeyConflict"
	managedSessionKeyReasonRotated              = "SessionKeyRotated"
	managedSessionKeyReasonRotationFailed       = "SessionKeyRotationFailed"
	sessionKeyConditionReasonConflict           = "SessionKeyConflict"
	sessionKeyConditionReasonLifecycle          = "SessionKeyLifecycleError"
	managedSessionKeyMaxPreviousKeys            = 1
)

var managedSessionKeyOperationsTotal = metrics.NewCounter(
	metrics.CounterOpts{
		Subsystem: "deployer",
		Name:      "managed_session_key_operations_total",
		Help:      "Total number of managed session key lifecycle operations by action and result.",
	},
	[]string{"action", "result"},
)

type managedSessionKeyring struct {
	Version   string   `json:"version"`
	Primary   string   `json:"primary"`
	Previous  []string `json:"previous,omitempty"`
	RotatedAt string   `json:"rotatedAt,omitempty"`
}

type SessionKeyConflictError struct {
	Gateway    types.NamespacedName
	SecretName string
	Message    string
}

func (e *SessionKeyConflictError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf(
		"session key secret conflict for Gateway %s/%s: Secret %s already exists but is not managed by this Gateway",
		e.Gateway.Namespace,
		e.Gateway.Name,
		e.SecretName,
	)
}

type SessionKeyLifecycleError struct {
	Gateway   types.NamespacedName
	Operation string
	Err       error
}

func (e *SessionKeyLifecycleError) Error() string {
	return fmt.Sprintf(
		"session key %s failed for Gateway %s/%s: %v",
		e.Operation,
		e.Gateway.Namespace,
		e.Gateway.Name,
		e.Err,
	)
}

func (e *SessionKeyLifecycleError) Unwrap() error {
	return e.Err
}

func SessionKeyConditionReason(err error) (string, bool) {
	var conflictErr *SessionKeyConflictError
	if errors.As(err, &conflictErr) {
		return sessionKeyConditionReasonConflict, true
	}

	var lifecycleErr *SessionKeyLifecycleError
	if errors.As(err, &lifecycleErr) {
		return sessionKeyConditionReasonLifecycle, true
	}

	return "", false
}

func IsSessionKeyConditionReason(reason string) bool {
	return reason == sessionKeyConditionReasonConflict || reason == sessionKeyConditionReasonLifecycle
}

func newSessionKeyEventRecorder(cli kube.Client) record.EventRecorder {
	eventBroadcaster := record.NewBroadcaster()
	eventRecorder := eventBroadcaster.NewRecorder(
		schemes.DefaultScheme(),
		corev1.EventSource{Component: wellknown.DefaultAgwControllerName},
	)
	eventBroadcaster.StartRecordingToSink(&typedcorev1.EventSinkImpl{
		Interface: cli.Kube().CoreV1().Events(""),
	})
	return eventRecorder
}

func recordManagedSessionKeyMetric(action, result string) {
	managedSessionKeyOperationsTotal.Inc(
		metrics.Label{Name: "action", Value: action},
		metrics.Label{Name: "result", Value: result},
	)
}

func generateSessionKey() (string, error) {
	var key [32]byte
	if _, err := rand.Read(key[:]); err != nil {
		return "", fmt.Errorf("failed to generate session key: %w", err)
	}
	return hex.EncodeToString(key[:]), nil
}

func validateSessionKey(key string) error {
	key = strings.TrimSpace(key)
	decoded, err := hex.DecodeString(key)
	if err != nil {
		return fmt.Errorf("invalid hex-encoded session key: %w", err)
	}
	if len(decoded) != 32 {
		return fmt.Errorf("invalid session key length: expected 32 bytes, got %d", len(decoded))
	}
	return nil
}

func newManagedSessionKeyring(sessionKeyGen func() (string, error)) (*managedSessionKeyring, error) {
	primary, err := sessionKeyGen()
	if err != nil {
		return nil, err
	}
	if err := validateSessionKey(primary); err != nil {
		return nil, fmt.Errorf("generated invalid session key: %w", err)
	}

	return &managedSessionKeyring{
		Version: managedSessionKeyVersion,
		Primary: primary,
	}, nil
}

func parseManagedSessionKeyring(payload []byte) (*managedSessionKeyring, error) {
	if len(payload) == 0 {
		return nil, errors.New("missing keyring payload")
	}

	var keyring managedSessionKeyring
	if err := json.Unmarshal(payload, &keyring); err != nil {
		return nil, fmt.Errorf("invalid JSON payload: %w", err)
	}
	if err := keyring.Validate(); err != nil {
		return nil, err
	}
	return &keyring, nil
}

func (k *managedSessionKeyring) Validate() error {
	if k == nil {
		return errors.New("missing keyring")
	}
	if k.Version != managedSessionKeyVersion {
		return fmt.Errorf("unsupported keyring version %q", k.Version)
	}
	if err := validateSessionKey(k.Primary); err != nil {
		return fmt.Errorf("invalid primary key: %w", err)
	}
	if len(k.Previous) > managedSessionKeyMaxPreviousKeys {
		return fmt.Errorf("too many previous keys: got %d, max %d", len(k.Previous), managedSessionKeyMaxPreviousKeys)
	}
	seen := map[string]struct{}{k.Primary: {}}
	for i, previous := range k.Previous {
		if err := validateSessionKey(previous); err != nil {
			return fmt.Errorf("invalid previous[%d] key: %w", i, err)
		}
		if _, exists := seen[previous]; exists {
			return fmt.Errorf("duplicate key material at previous[%d]", i)
		}
		seen[previous] = struct{}{}
	}
	if k.RotatedAt != "" {
		if _, err := time.Parse(time.RFC3339, k.RotatedAt); err != nil {
			return fmt.Errorf("invalid rotatedAt timestamp: %w", err)
		}
	}
	return nil
}

func (k *managedSessionKeyring) Serialize() ([]byte, error) {
	if err := k.Validate(); err != nil {
		return nil, err
	}
	payload, err := json.Marshal(k)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal keyring: %w", err)
	}
	return payload, nil
}

func (k *managedSessionKeyring) Rotated(sessionKeyGen func() (string, error), now time.Time) (*managedSessionKeyring, error) {
	next, err := newManagedSessionKeyring(sessionKeyGen)
	if err != nil {
		return nil, err
	}
	next.Previous = append(next.Previous, k.Primary)
	next.RotatedAt = now.UTC().Format(time.RFC3339)
	return next, nil
}

func (g *agentgatewayParametersHelmValuesGenerator) buildSessionKeySecret(
	ctx context.Context,
	gw *gwv1.Gateway,
	secretName string,
) (*corev1.Secret, error) {
	return g.ensureManagedSessionKeySecret(ctx, gw, secretName)
}

func (g *agentgatewayParametersHelmValuesGenerator) ensureManagedSessionKeySecret(
	ctx context.Context,
	gw *gwv1.Gateway,
	secretName string,
) (*corev1.Secret, error) {
	liveSecret, err := g.apiClient.Kube().CoreV1().Secrets(gw.Namespace).Get(ctx, secretName, metav1.GetOptions{})
	switch {
	case apierrors.IsNotFound(err):
		return g.createManagedSessionKeySecret(ctx, gw, secretName)
	case err != nil:
		return nil, &SessionKeyLifecycleError{
			Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
			Operation: "read",
			Err:       err,
		}
	}

	return g.resolveManagedSessionKeySecret(ctx, gw, liveSecret)
}

func (g *agentgatewayParametersHelmValuesGenerator) createManagedSessionKeySecret(
	ctx context.Context,
	gw *gwv1.Gateway,
	secretName string,
) (*corev1.Secret, error) {
	keyring, err := newManagedSessionKeyring(g.sessionKeyGen)
	if err != nil {
		recordManagedSessionKeyMetric("create", "error")
		return nil, &SessionKeyLifecycleError{
			Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
			Operation: "create",
			Err:       err,
		}
	}

	desired, err := managedSessionKeySecretForGateway(gw, secretName, nil, keyring, "")
	if err != nil {
		recordManagedSessionKeyMetric("create", "error")
		return nil, &SessionKeyLifecycleError{
			Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
			Operation: "create",
			Err:       err,
		}
	}

	_, err = g.apiClient.Kube().CoreV1().Secrets(gw.Namespace).Create(ctx, desired, metav1.CreateOptions{})
	if apierrors.IsAlreadyExists(err) {
		return g.ensureManagedSessionKeySecret(ctx, gw, secretName)
	}
	if err != nil {
		recordManagedSessionKeyMetric("create", "error")
		return nil, &SessionKeyLifecycleError{
			Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
			Operation: "create",
			Err:       err,
		}
	}

	recordManagedSessionKeyMetric("create", "success")
	if g.eventRecorder != nil {
		g.eventRecorder.Eventf(
			gw,
			corev1.EventTypeNormal,
			managedSessionKeyReasonCreated,
			"created managed session key Secret %s/%s",
			gw.Namespace,
			secretName,
		)
	}
	return desired, nil
}

func (g *agentgatewayParametersHelmValuesGenerator) resolveManagedSessionKeySecret(
	ctx context.Context,
	gw *gwv1.Gateway,
	liveSecret *corev1.Secret,
) (*corev1.Secret, error) {
	if conflictMessage := managedSessionKeyConflictMessage(liveSecret, gw); conflictMessage != "" {
		recordManagedSessionKeyMetric("conflict", "error")
		if g.eventRecorder != nil {
			g.eventRecorder.Eventf(gw, corev1.EventTypeWarning, managedSessionKeyReasonConflict, conflictMessage)
		}
		return nil, &SessionKeyConflictError{
			Gateway:    types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
			SecretName: liveSecret.Name,
			Message:    conflictMessage,
		}
	}

	keyring, parseErr := parseManagedSessionKeyring(liveSecret.Data[managedSessionKeyDataKey])
	rotationToken := strings.TrimSpace(gw.Annotations[managedSessionKeyRotationAnnotation])
	handledToken := strings.TrimSpace(liveSecret.Annotations[managedSessionKeyHandledRotationAnnotation])
	needsRotate := parseErr == nil && rotationToken != "" && rotationToken != handledToken

	needsRepair := parseErr != nil || managedSessionKeyMetadataNeedsRepair(liveSecret, gw)
	switch {
	case needsRotate:
		rotatedKeyring, err := keyring.Rotated(g.sessionKeyGen, time.Now())
		if err != nil {
			recordManagedSessionKeyMetric("rotate", "error")
			if g.eventRecorder != nil {
				g.eventRecorder.Eventf(gw, corev1.EventTypeWarning, managedSessionKeyReasonRotationFailed, "failed rotating managed session key Secret %s/%s: %v", gw.Namespace, liveSecret.Name, err)
			}
			return nil, &SessionKeyLifecycleError{
				Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
				Operation: "rotate",
				Err:       err,
			}
		}
		updated, err := g.updateManagedSessionKeySecret(ctx, gw, liveSecret.Name, rotatedKeyring, rotationToken)
		if err != nil {
			recordManagedSessionKeyMetric("rotate", "error")
			if g.eventRecorder != nil {
				g.eventRecorder.Eventf(gw, corev1.EventTypeWarning, managedSessionKeyReasonRotationFailed, "failed rotating managed session key Secret %s/%s: %v", gw.Namespace, liveSecret.Name, err)
			}
			return nil, err
		}
		recordManagedSessionKeyMetric("rotate", "success")
		if g.eventRecorder != nil {
			g.eventRecorder.Eventf(gw, corev1.EventTypeNormal, managedSessionKeyReasonRotated, "rotated managed session key Secret %s/%s", gw.Namespace, liveSecret.Name)
		}
		return updated, nil
	case needsRepair:
		repairedKeyring := keyring
		if parseErr != nil {
			var repairErr error
			repairedKeyring, repairErr = newManagedSessionKeyring(g.sessionKeyGen)
			if repairErr != nil {
				recordManagedSessionKeyMetric("repair", "error")
				return nil, &SessionKeyLifecycleError{
					Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
					Operation: "repair",
					Err:       repairErr,
				}
			}
		}
		updated, err := g.updateManagedSessionKeySecret(ctx, gw, liveSecret.Name, repairedKeyring, handledToken)
		if err != nil {
			recordManagedSessionKeyMetric("repair", "error")
			return nil, err
		}
		recordManagedSessionKeyMetric("repair", "success")
		if g.eventRecorder != nil {
			g.eventRecorder.Eventf(gw, corev1.EventTypeNormal, managedSessionKeyReasonRepaired, "repaired managed session key Secret %s/%s", gw.Namespace, liveSecret.Name)
		}
		return updated, nil
	default:
		return managedSessionKeySecretForGateway(gw, liveSecret.Name, liveSecret, keyring, handledToken)
	}
}

func (g *agentgatewayParametersHelmValuesGenerator) updateManagedSessionKeySecret(
	ctx context.Context,
	gw *gwv1.Gateway,
	secretName string,
	keyring *managedSessionKeyring,
	handledRotationToken string,
) (*corev1.Secret, error) {
	var updated *corev1.Secret
	err := utilretry.RetryOnConflict(utilretry.DefaultRetry, func() error {
		liveSecret, err := g.apiClient.Kube().CoreV1().Secrets(gw.Namespace).Get(ctx, secretName, metav1.GetOptions{})
		if err != nil {
			return err
		}
		if conflictMessage := managedSessionKeyConflictMessage(liveSecret, gw); conflictMessage != "" {
			return &SessionKeyConflictError{
				Gateway:    types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
				SecretName: secretName,
				Message:    conflictMessage,
			}
		}

		desired, err := managedSessionKeySecretForGateway(gw, secretName, liveSecret, keyring, handledRotationToken)
		if err != nil {
			return err
		}
		result, err := g.apiClient.Kube().CoreV1().Secrets(gw.Namespace).Update(ctx, desired, metav1.UpdateOptions{})
		if err != nil {
			return err
		}
		updated, err = managedSessionKeySecretForGateway(gw, secretName, result, keyring, handledRotationToken)
		return err
	})
	if err != nil {
		var conflictErr *SessionKeyConflictError
		if errors.As(err, &conflictErr) {
			recordManagedSessionKeyMetric("conflict", "error")
			return nil, err
		}
		return nil, &SessionKeyLifecycleError{
			Gateway:   types.NamespacedName{Namespace: gw.Namespace, Name: gw.Name},
			Operation: "update",
			Err:       err,
		}
	}
	return updated, nil
}

func managedSessionKeySecretForGateway(
	gw *gwv1.Gateway,
	secretName string,
	base *corev1.Secret,
	keyring *managedSessionKeyring,
	handledRotationToken string,
) (*corev1.Secret, error) {
	payload, err := keyring.Serialize()
	if err != nil {
		return nil, err
	}

	var secret *corev1.Secret
	if base != nil {
		secret = base.DeepCopy()
	} else {
		secret = &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{
				Name:      secretName,
				Namespace: gw.Namespace,
			},
		}
	}
	secret.TypeMeta = metav1.TypeMeta{
		APIVersion: corev1.SchemeGroupVersion.String(),
		Kind:       "Secret",
	}

	if secret.Labels == nil {
		secret.Labels = map[string]string{}
	}
	secret.Labels[managedSessionKeyLabel] = managedSessionKeyLabelValue
	secret.Labels[wellknown.GatewayNameLabel] = safeLabelValue(gw.Name)
	secret.Labels[wellknown.GatewayClassNameLabel] = string(gw.Spec.GatewayClassName)

	if secret.Annotations == nil {
		secret.Annotations = map[string]string{}
	}
	secret.Annotations[managedSessionKeyGatewayNameAnnotation] = gw.Name
	secret.Annotations[managedSessionKeyGatewayNamespaceAnnotation] = gw.Namespace
	secret.Annotations[managedSessionKeyGatewayUIDAnnotation] = string(gw.UID)
	if handledRotationToken != "" {
		secret.Annotations[managedSessionKeyHandledRotationAnnotation] = handledRotationToken
	} else {
		delete(secret.Annotations, managedSessionKeyHandledRotationAnnotation)
	}

	secret.Name = secretName
	secret.Namespace = gw.Namespace
	secret.Type = corev1.SecretTypeOpaque
	secret.Data = map[string][]byte{
		managedSessionKeyDataKey: payload,
	}
	return normalizeSecretForApply(secret), nil
}

func normalizeSecretForApply(secret *corev1.Secret) *corev1.Secret {
	secret.ResourceVersion = ""
	secret.Generation = 0
	secret.UID = ""
	secret.CreationTimestamp = metav1.Time{}
	secret.DeletionTimestamp = nil
	secret.DeletionGracePeriodSeconds = nil
	secret.ManagedFields = nil
	return secret
}

func managedSessionKeyMetadataNeedsRepair(secret *corev1.Secret, gw *gwv1.Gateway) bool {
	if secret.Labels == nil || secret.Annotations == nil {
		return true
	}
	if secret.Labels[managedSessionKeyLabel] != managedSessionKeyLabelValue {
		return true
	}
	if secret.Annotations[managedSessionKeyGatewayNameAnnotation] != gw.Name {
		return true
	}
	if secret.Annotations[managedSessionKeyGatewayNamespaceAnnotation] != gw.Namespace {
		return true
	}
	return secret.Annotations[managedSessionKeyGatewayUIDAnnotation] != string(gw.UID)
}

func managedSessionKeyConflictMessage(secret *corev1.Secret, gw *gwv1.Gateway) string {
	managedValue := secret.Labels[managedSessionKeyLabel]
	switch {
	case managedValue == "":
		return fmt.Sprintf(
			"session key Secret %s/%s already exists but is not controller-managed",
			secret.Namespace,
			secret.Name,
		)
	case managedValue != managedSessionKeyLabelValue:
		return fmt.Sprintf(
			"session key Secret %s/%s already exists with unexpected managed label %q",
			secret.Namespace,
			secret.Name,
			managedValue,
		)
	}

	if gatewayName := secret.Annotations[managedSessionKeyGatewayNameAnnotation]; gatewayName != "" && gatewayName != gw.Name {
		return fmt.Sprintf(
			"session key Secret %s/%s is bound to Gateway %s/%s",
			secret.Namespace,
			secret.Name,
			secret.Namespace,
			gatewayName,
		)
	}
	if gatewayNamespace := secret.Annotations[managedSessionKeyGatewayNamespaceAnnotation]; gatewayNamespace != "" && gatewayNamespace != gw.Namespace {
		return fmt.Sprintf(
			"session key Secret %s/%s is bound to namespace %s",
			secret.Namespace,
			secret.Name,
			gatewayNamespace,
		)
	}
	if gatewayUID := secret.Annotations[managedSessionKeyGatewayUIDAnnotation]; gatewayUID != "" && gatewayUID != string(gw.UID) {
		return fmt.Sprintf(
			"session key Secret %s/%s is bound to a different Gateway UID",
			secret.Namespace,
			secret.Name,
		)
	}
	return ""
}

func sessionKeyChecksum(secret *corev1.Secret) (string, error) {
	payload, found := secret.Data[managedSessionKeyDataKey]
	if !found || len(payload) == 0 {
		return "", fmt.Errorf("session key secret %s/%s missing %s entry", secret.Namespace, secret.Name, managedSessionKeyDataKey)
	}

	checksum := sha256.Sum256(payload)
	return hex.EncodeToString(checksum[:]), nil
}
