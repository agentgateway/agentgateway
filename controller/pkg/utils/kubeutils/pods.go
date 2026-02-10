package kubeutils

import (
	"context"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/fields"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// GetPodsForDeployment gets all pods backing a deployment that are running and ready
func GetPodsForDeployment(
	ctx context.Context,
	restConfig *rest.Config,
	deploymentName string,
	deploymentNamespace string,
) ([]string, error) {
	kubeClient, err := kubernetes.NewForConfig(restConfig)
	if err != nil {
		return nil, err
	}

	// We change the implementation of this method, to return only pods that are ready
	// This is done to reduce the chance that a developer misuses this utility
	// If you want to get pods that are not ready, you can use GetPodsForDeploymentWithPredicate
	return GetReadyPodsForDeployment(
		ctx,
		kubeClient,
		metav1.ObjectMeta{
			Name:      deploymentName,
			Namespace: deploymentNamespace,
		})
}

// GetReadyPodsForDeployment gets all pods backing a deployment that are running and ready
// This function should be preferred over GetPodsForDeployment
func GetReadyPodsForDeployment(
	ctx context.Context,
	kubeClient *kubernetes.Clientset,
	deploy metav1.ObjectMeta,
) ([]string, error) {
	// This predicate will return true if and only if the pod is ready
	readyPodPredicate := func(pod corev1.Pod) bool {
		for _, condition := range pod.Status.Conditions {
			if condition.Type == corev1.PodReady {
				return true
			}
		}
		return false
	}

	return GetPodsForDeploymentWithPredicate(ctx, kubeClient, deploy, readyPodPredicate)
}

// GetPodsForDeploymentWithPredicate gets all pods backing a deployment that are running and satisfy the predicate function
func GetPodsForDeploymentWithPredicate(
	ctx context.Context,
	kubeClient *kubernetes.Clientset,
	deploy metav1.ObjectMeta,
	predicate func(pod corev1.Pod) bool,
) ([]string, error) {
	deployment, err := kubeClient.AppsV1().Deployments(deploy.GetNamespace()).Get(ctx, deploy.GetName(), metav1.GetOptions{})
	if err != nil {
		return nil, err
	}
	matchLabels := deployment.Spec.Selector.MatchLabels
	listOptions := (&client.ListOptions{
		LabelSelector: labels.SelectorFromSet(matchLabels),
		FieldSelector: fields.Set{"status.phase": "Running"}.AsSelector(),
	}).AsListOptions()

	podList, err := kubeClient.CoreV1().Pods(deploy.GetNamespace()).List(ctx, *listOptions)
	if err != nil {
		return nil, err
	}

	podNames := make([]string, 0, len(podList.Items))
	for _, pod := range podList.Items {
		if predicate(pod) {
			podNames = append(podNames, pod.Name)
		}
	}

	return podNames, nil
}

// GetPodsForService gets all pods backing a deployment
func GetPodsForService(
	ctx context.Context,
	restConfig *rest.Config,
	serviceName string,
	serviceNamespace string,
) ([]string, error) {
	kubeClient, err := kubernetes.NewForConfig(restConfig)
	if err != nil {
		return nil, err
	}

	service, err := kubeClient.CoreV1().Services(serviceNamespace).Get(ctx, serviceName, metav1.GetOptions{})
	if err != nil {
		return nil, err
	}

	matchLabels := service.Spec.Selector
	listOptions := (&client.ListOptions{LabelSelector: labels.SelectorFromSet(matchLabels)}).AsListOptions()

	podList, err := kubeClient.CoreV1().Pods(serviceNamespace).List(ctx, *listOptions)
	if err != nil {
		return nil, err
	}

	pods := make([]string, len(podList.Items))
	for i := range podList.Items {
		pods[i] = podList.Items[i].Name
	}

	return pods, nil
}
