package wellknown

import (
	appsv1 "k8s.io/api/apps/v1"
	autoscalingv2 "k8s.io/api/autoscaling/v2"
	corev1 "k8s.io/api/core/v1"
	policyv1 "k8s.io/api/policy/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
)

var (
	DeploymentGVK              = appsv1.SchemeGroupVersion.WithKind("Deployment")
	SecretGVK                  = corev1.SchemeGroupVersion.WithKind("Secret")
	ConfigMapGVK               = corev1.SchemeGroupVersion.WithKind("ConfigMap")
	ServiceGVK                 = corev1.SchemeGroupVersion.WithKind("Service")
	ServiceAccountGVK          = corev1.SchemeGroupVersion.WithKind("ServiceAccount")
	ClusterRoleBindingGVK      = rbacv1.SchemeGroupVersion.WithKind("ClusterRoleBinding")
	ClusterRoleGVK             = rbacv1.SchemeGroupVersion.WithKind("ClusterRole")
	PodDisruptionBudgetGVK     = policyv1.SchemeGroupVersion.WithKind("PodDisruptionBudget")
	HorizontalPodAutoscalerGVK = autoscalingv2.SchemeGroupVersion.WithKind("HorizontalPodAutoscaler")
	// VerticalPodAutoscaler is from the autoscaling.k8s.io API group (VPA custom resource)
	VerticalPodAutoscalerGVK = schema.GroupVersionKind{Group: "autoscaling.k8s.io", Version: "v1", Kind: "VerticalPodAutoscaler"}
)
