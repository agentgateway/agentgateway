//go:build e2e

package assertions

import (
	"context"
	"fmt"
	"time"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"

	. "github.com/onsi/gomega"
)

func (p *Provider) EventuallyObjectsExist(ctx context.Context, objects ...client.Object) {
	p.t.Helper()
	for _, o := range objects {
		p.Gomega.Eventually(ctx, func(innerG Gomega) {
			err := p.clusterContext.Client.Get(ctx, client.ObjectKeyFromObject(o), o)
			innerG.Expect(err).NotTo(HaveOccurred(), "object %s %s should be available in cluster", o.GetObjectKind().GroupVersionKind().String(), client.ObjectKeyFromObject(o).String())
		}).
			WithContext(ctx).
			WithTimeout(time.Second*20).
			WithPolling(time.Millisecond*200).
			Should(Succeed(), fmt.Sprintf("object %s %s should be available in cluster", o.GetObjectKind().GroupVersionKind().String(), client.ObjectKeyFromObject(o).String()))
	}
}

func (p *Provider) EventuallyObjectsNotExist(ctx context.Context, objects ...client.Object) {
	p.t.Helper()
	for _, o := range objects {
		p.Gomega.Eventually(ctx, func(innerG Gomega) {
			err := p.clusterContext.Client.Get(ctx, client.ObjectKeyFromObject(o), o)
			innerG.Expect(apierrors.IsNotFound(err)).To(BeTrue(), "object %s %s should not be found in cluster", o.GetObjectKind().GroupVersionKind().String(), client.ObjectKeyFromObject(o).String())
		}).
			WithContext(ctx).
			WithTimeout(time.Second*60).
			WithPolling(time.Millisecond*200).
			Should(Succeed(), fmt.Sprintf("object %s %s should not be found in cluster", o.GetObjectKind().GroupVersionKind().String(), client.ObjectKeyFromObject(o).String()))
	}
}

// EventuallyObjectTypesNotExist asserts that eventually no objects of the specified types exist on the cluster.
// The `objectLists` holds the object list types to check, e.g. to check that no HTTPRoutes exist on the cluster, pass in HTTPRouteList{}
func (p *Provider) EventuallyObjectTypesNotExist(ctx context.Context, objectLists ...client.ObjectList) {
	p.t.Helper()
	for _, o := range objectLists {
		p.Gomega.Eventually(ctx, func(innerG Gomega) {
			err := p.clusterContext.Client.List(ctx, o)
			p.Assert.NoError(err, "can list %T", o)
			innerG.Expect(o).To(HaveField("Items", HaveLen(0)))
		}).
			WithContext(ctx).
			WithTimeout(time.Second*20).
			WithPolling(time.Millisecond*200).
			Should(Succeed(), fmt.Sprintf("object type %T should not be found in cluster", o))
	}
}

func (p *Provider) ConsistentlyObjectsNotExist(ctx context.Context, objects ...client.Object) {
	p.t.Helper()
	for _, o := range objects {
		p.Gomega.Consistently(ctx, func(innerG Gomega) {
			err := p.clusterContext.Client.Get(ctx, client.ObjectKeyFromObject(o), o)
			innerG.Expect(apierrors.IsNotFound(err)).To(BeTrue(), "object %s %s should not be found in cluster", o.GetObjectKind().GroupVersionKind().String(), client.ObjectKeyFromObject(o).String())
		}).
			WithContext(ctx).
			WithTimeout(time.Second*10).
			WithPolling(time.Second*1).
			Should(Succeed(), fmt.Sprintf("object %s %s should not be found in cluster", o.GetObjectKind().GroupVersionKind().String(), client.ObjectKeyFromObject(o).String()))
	}
}

func (p *Provider) ExpectNamespaceNotExist(ctx context.Context, ns string) {
	p.t.Helper()
	_, err := p.clusterContext.Clientset.CoreV1().Namespaces().Get(ctx, ns, metav1.GetOptions{})
	p.Gomega.Expect(apierrors.IsNotFound(err)).To(BeTrue(), fmt.Sprintf("namespace %s should not be found in cluster", ns))
}

func (p *Provider) EventuallyNamespaceExists(ctx context.Context, ns string) {
	p.t.Helper()
	p.Gomega.Eventually(ctx, func(innerG Gomega) {
		_, err := p.clusterContext.Clientset.CoreV1().Namespaces().Get(ctx, ns, metav1.GetOptions{})
		innerG.Expect(err).NotTo(HaveOccurred(), "namespace %s should exist", ns)
	}).
		WithContext(ctx).
		WithTimeout(time.Second*20).
		WithPolling(time.Millisecond*200).
		Should(Succeed(), fmt.Sprintf("namespace %s should exist", ns))
}

func (p *Provider) ExpectObjectDeleted(manifest string, err error, actualOutput string) {
	p.t.Helper()
	p.Assert.NoError(err, "can delete "+manifest)
}
