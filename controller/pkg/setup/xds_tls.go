package setup

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/subtle"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"fmt"
	"math"
	"math/big"
	"net"
	"sync"
	"time"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/kubetypes"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/fields"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
)

const (
	xdsCACertKey = "ca.crt"
	xdsCAKeyKey  = "ca.key"
	xdsCertKey   = "tls.crt"
	xdsKeyKey    = "tls.key"

	xdsCACertLifetime      = 10 * 365 * 24 * time.Hour
	xdsLeafCertLifetime    = 24 * time.Hour
	xdsLeafCertRenewBefore = 12 * time.Hour
)

type xdsTLSMaterial struct {
	mu          sync.RWMutex
	currentCert *tls.Certificate
	callback    func(tls.Certificate)
}

func setupXdsTLSMaterial(ctx context.Context, cli apiclient.Client, namespace, name string, hosts []string) (*xdsTLSMaterial, error) {
	material := &xdsTLSMaterial{}
	s := &xdsTLSMaterialSyncer{
		ctx:       ctx,
		cli:       cli,
		namespace: namespace,
		name:      name,
		hosts:     hosts,
		material:  material,
	}
	s.queue = controllers.NewQueue("XdsTLSMaterialController", controllers.WithReconciler(s.reconcile), controllers.WithMaxAttempts(math.MaxInt))
	secret, err := ensureXdsSecret(ctx, cli, namespace, name)
	if err != nil {
		return nil, err
	}
	if err := s.refreshSecret(secret, true); err != nil {
		return nil, err
	}
	inf := kclient.NewFiltered[*corev1.Secret](cli, kubetypes.Filter{
		FieldSelector: fields.OneTermEqualSelector("metadata.name", name).String(),
		Namespace:     namespace,
	})
	s.secrets = inf

	handler := controllers.TypedObjectHandler[*corev1.Secret](func(o *corev1.Secret) {
		s.queue.AddObject(o)
	})
	inf.AddEventHandler(handler)
	inf.Start(ctx.Done())
	if !cli.WaitForCacheSync("xDS TLS Secret", ctx.Done(), inf.HasSynced) {
		s.queue.ShutDownEarly()
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		return nil, fmt.Errorf("failed to sync xDS TLS secret informer")
	}
	go s.queue.Run(ctx.Done())
	return material, nil
}

type xdsTLSMaterialSyncer struct {
	ctx       context.Context
	cli       apiclient.Client
	namespace string
	name      string
	hosts     []string
	material  *xdsTLSMaterial
	secrets   kclient.Client[*corev1.Secret]
	queue     controllers.Queue

	lastRV       string
	cancelRenew  context.CancelFunc
	renewalTimer *time.Timer
}

func (s *xdsTLSMaterialSyncer) reconcile(req types.NamespacedName) error {
	if req.Namespace != s.namespace || req.Name != s.name {
		return nil
	}
	secret := s.secrets.Get(s.name, s.namespace)
	if secret == nil {
		var err error
		secret, err = ensureXdsSecret(s.ctx, s.cli, s.namespace, s.name)
		if err != nil {
			return err
		}
	}
	return s.refreshSecret(secret, false)
}

func (s *xdsTLSMaterialSyncer) refreshSecret(secret *corev1.Secret, force bool) error {
	if !force && !s.material.shouldRefresh(secret, s.lastRV) {
		return nil
	}
	certPEM, keyPEM, err := extractServingMaterial(secret, s.hosts)
	if err != nil {
		return err
	}
	cert, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		return err
	}
	cert.Leaf, err = parseCertificate(certPEM)
	if err != nil {
		return err
	}
	s.material.setCertificate(cert)
	s.lastRV = secret.ResourceVersion
	s.scheduleRenewal(secret, cert)
	return nil
}

func (s *xdsTLSMaterialSyncer) scheduleRenewal(secret *corev1.Secret, cert tls.Certificate) {
	if s.cancelRenew != nil {
		s.cancelRenew()
		s.cancelRenew = nil
	}
	if s.renewalTimer != nil {
		s.renewalTimer.Stop()
		s.renewalTimer = nil
	}
	if !secretNeedsGeneratedLeaf(secret) || cert.Leaf == nil {
		return
	}
	renewCtx, cancel := context.WithCancel(s.ctx)
	s.cancelRenew = cancel
	delay := max(time.Until(cert.Leaf.NotAfter.Add(-xdsLeafCertRenewBefore)), 0)
	s.renewalTimer = time.AfterFunc(delay, func() {
		select {
		case <-renewCtx.Done():
			return
		default:
		}
		s.queue.Add(types.NamespacedName{Namespace: s.namespace, Name: s.name})
	})
}

func (m *xdsTLSMaterial) GetCertificate(_ *tls.ClientHelloInfo) (*tls.Certificate, error) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.currentCert, nil
}

func (m *xdsTLSMaterial) RegisterCallback(callback func(tls.Certificate)) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.currentCert != nil {
		callback(*m.currentCert)
	}
	m.callback = callback
}

func (m *xdsTLSMaterial) setCertificate(cert tls.Certificate) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.currentCert = &cert
	if m.callback != nil {
		m.callback(cert)
	}
}

func (m *xdsTLSMaterial) shouldRefresh(secret *corev1.Secret, lastRV string) bool {
	if secret.ResourceVersion != lastRV {
		return true
	}
	if !secretNeedsGeneratedLeaf(secret) {
		return false
	}
	m.mu.RLock()
	defer m.mu.RUnlock()
	if m.currentCert == nil || m.currentCert.Leaf == nil {
		return true
	}
	return time.Until(m.currentCert.Leaf.NotAfter) <= xdsLeafCertRenewBefore
}

func ensureXdsSecret(ctx context.Context, cli apiclient.Client, ns, name string) (*corev1.Secret, error) {
	secrets := cli.Kube().CoreV1().Secrets(ns)
	s, err := secrets.Get(ctx, name, metav1.GetOptions{})
	if err == nil {
		return s, nil
	}
	if !apierrors.IsNotFound(err) {
		return nil, err
	}
	caCert, caKey, err := generateCA("agw-xds-ca")
	if err != nil {
		return nil, err
	}
	toCreate := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns},
		Type:       corev1.SecretTypeOpaque,
		Data: map[string][]byte{
			xdsCACertKey: caCert,
			xdsCAKeyKey:  caKey,
		},
	}
	res, err := secrets.Create(ctx, toCreate, metav1.CreateOptions{})
	if err != nil {
		if apierrors.IsAlreadyExists(err) {
			return secrets.Get(ctx, name, metav1.GetOptions{})
		}
		return nil, err
	}
	return res, nil
}

func parseCertificate(certPEM []byte) (*x509.Certificate, error) {
	block, _ := pem.Decode(certPEM)
	if block == nil {
		return nil, fmt.Errorf("failed to parse certificate PEM")
	}
	return x509.ParseCertificate(block.Bytes)
}

func extractServingMaterial(secret *corev1.Secret, hosts []string) ([]byte, []byte, error) {
	switch {
	case len(secret.Data[xdsCACertKey]) > 0 && len(secret.Data[xdsCAKeyKey]) > 0:
		return generateLeafFromCA(secret.Data[xdsCACertKey], secret.Data[xdsCAKeyKey], hosts)
	case len(secret.Data[xdsCertKey]) > 0 || len(secret.Data[xdsKeyKey]) > 0:
		cert := secret.Data[xdsCertKey]
		key := secret.Data[xdsKeyKey]
		if len(cert) == 0 || len(key) == 0 {
			return nil, nil, fmt.Errorf("xDS secret %s/%s must include both tls.crt and tls.key", secret.Namespace, secret.Name)
		}
		if certNeedsGeneratedLeaf(cert) {
			return generateLeafFromCA(cert, key, hosts)
		}
		if len(secret.Data[xdsCACertKey]) == 0 {
			return nil, nil, fmt.Errorf("xDS secret %s/%s with serving tls.crt/tls.key must include ca.crt", secret.Namespace, secret.Name)
		}
		return cert, key, nil
	default:
		return nil, nil, fmt.Errorf("xDS secret %s/%s must contain either ca.crt/ca.key, CA tls.crt/tls.key, or serving tls.crt/tls.key/ca.crt", secret.Namespace, secret.Name)
	}
}

func secretNeedsGeneratedLeaf(secret *corev1.Secret) bool {
	return len(secret.Data[xdsCAKeyKey]) > 0 || certNeedsGeneratedLeaf(secret.Data[xdsCertKey])
}

func certNeedsGeneratedLeaf(certPEM []byte) bool {
	cert, err := parseCertificate(certPEM)
	return err == nil && isSigningCA(cert)
}

func isSigningCA(cert *x509.Certificate) bool {
	return cert.IsCA && cert.KeyUsage&x509.KeyUsageCertSign != 0
}

func generateCA(commonName string) ([]byte, []byte, error) {
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, nil, err
	}
	serial, _ := rand.Int(rand.Reader, big.NewInt(1<<62))
	tpl := &x509.Certificate{
		SerialNumber: serial,
		Subject: pkix.Name{
			CommonName: commonName,
		},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(xdsCACertLifetime),
		IsCA:                  true,
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
	}
	der, err := x509.CreateCertificate(rand.Reader, tpl, tpl, &priv.PublicKey, priv)
	if err != nil {
		return nil, nil, err
	}
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	keyDER, err := x509.MarshalPKCS8PrivateKey(priv)
	if err != nil {
		return nil, nil, err
	}
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})
	return certPEM, keyPEM, nil
}

func generateLeafFromCA(caPEM, caKeyPEM []byte, hosts []string) ([]byte, []byte, error) {
	caBlock, _ := pem.Decode(caPEM)
	if caBlock == nil {
		return nil, nil, fmt.Errorf("failed to parse CA certificate PEM")
	}
	caCert, err := x509.ParseCertificate(caBlock.Bytes)
	if err != nil {
		return nil, nil, err
	}
	if !isSigningCA(caCert) {
		return nil, nil, fmt.Errorf("CA certificate is not allowed to sign certificates")
	}
	caKey, err := parsePrivateKey(caKeyPEM)
	if err != nil {
		return nil, nil, err
	}
	if !publicKeysEqual(caCert.PublicKey, caKey.Public()) {
		return nil, nil, fmt.Errorf("CA certificate and key do not match")
	}
	leafKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, nil, err
	}
	serial, _ := rand.Int(rand.Reader, big.NewInt(1<<62))
	tpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: "agw-xds-server"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(xdsLeafCertLifetime),
		KeyUsage:     x509.KeyUsageDigitalSignature,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}
	for _, host := range hosts {
		if host == "" {
			continue
		}
		if ip := net.ParseIP(host); ip != nil {
			tpl.IPAddresses = append(tpl.IPAddresses, ip)
			continue
		}
		tpl.DNSNames = append(tpl.DNSNames, host)
	}
	tpl.DNSNames = append(tpl.DNSNames, "localhost")
	der, err := x509.CreateCertificate(rand.Reader, tpl, caCert, &leafKey.PublicKey, caKey)
	if err != nil {
		return nil, nil, err
	}
	keyPEM, err := encodePrivateKey(leafKey)
	if err != nil {
		return nil, nil, err
	}
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), keyPEM, nil
}

func parsePrivateKey(keyPEM []byte) (crypto.Signer, error) {
	keyBlock, _ := pem.Decode(keyPEM)
	if keyBlock == nil {
		return nil, fmt.Errorf("failed to parse private key PEM")
	}
	if key, err := x509.ParsePKCS8PrivateKey(keyBlock.Bytes); err == nil {
		return asSigner(key)
	}
	if key, err := x509.ParsePKCS1PrivateKey(keyBlock.Bytes); err == nil {
		return key, nil
	}
	if key, err := x509.ParseECPrivateKey(keyBlock.Bytes); err == nil {
		return key, nil
	}
	return nil, fmt.Errorf("failed to parse private key")
}

func asSigner(key any) (crypto.Signer, error) {
	switch k := key.(type) {
	case *rsa.PrivateKey:
		return k, nil
	case *ecdsa.PrivateKey:
		return k, nil
	case ed25519.PrivateKey:
		return k, nil
	case crypto.Signer:
		return k, nil
	default:
		return nil, fmt.Errorf("unsupported private key type %T", key)
	}
}

func publicKeysEqual(a, b any) bool {
	aDER, err := x509.MarshalPKIXPublicKey(a)
	if err != nil {
		return false
	}
	bDER, err := x509.MarshalPKIXPublicKey(b)
	if err != nil {
		return false
	}
	return subtle.ConstantTimeCompare(aDER, bDER) == 1
}

func encodePrivateKey(key any) ([]byte, error) {
	keyDER, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		return nil, err
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER}),
		nil
}
