package matchers

import (
	"fmt"
	"log"
	"strings"

	"github.com/onsi/gomega/types"
	corev1 "k8s.io/api/core/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// ExpectedPod is a struct that represents the expected pod.
type ExpectedPod struct {
	// ContainerName is the name of the container. Optional.
	ContainerName string

	// Status is the pod phase status (e.g. Running, Pending, Succeeded, Failed). Optional.
	Status corev1.PodPhase

	// Ready indicates that the Pod is able to serve requests. Optional.
	Ready bool

	// TODO(npolshak): Add more fields to match on as needed
}

// PodMatches returns a GomegaMatcher that checks whether a pod
func PodMatches(pod ExpectedPod) types.GomegaMatcher {
	return &podMatcher{expectedPod: pod}
}

type podMatcher struct {
	expectedPod ExpectedPod
}

func (pm *podMatcher) Match(actual any) (bool, error) {
	pod, ok := actual.(corev1.Pod)
	if !ok {
		return false, fmt.Errorf("expected a pod, got %T", actual)
	}
	if pm.expectedPod.ContainerName != "" {
		foundContainer := false
		for _, container := range pod.Spec.Containers {
			if container.Name == pm.expectedPod.ContainerName {
				foundContainer = true
			}
		}
		if !foundContainer {
			log.Printf("expected pod %s to have container '%s', but it was not found", client.ObjectKeyFromObject(&pod), pm.expectedPod.ContainerName)
			return false, nil
		}
	}

	if pm.expectedPod.Status != "" {
		if pod.Status.Phase != pm.expectedPod.Status {
			log.Printf("expected pod %s to have status %s, but it was %s", client.ObjectKeyFromObject(&pod), pm.expectedPod.Status, pod.Status.Phase)
			return false, nil
		}
	}

	if pm.expectedPod.Ready {
		for _, condition := range pod.Status.Conditions {
			if condition.Type == corev1.PodReady {
				ready := condition.Status == corev1.ConditionTrue
				if !ready {
					log.Printf("expected pod %s to have condition ready, but it was not ready", client.ObjectKeyFromObject(&pod))
				}
				return ready, nil
			}
		}
		log.Printf("expected pod %s to have condition ready, but it was not found", client.ObjectKeyFromObject(&pod))
		return false, nil
	}

	return true, nil
}

func (pm *podMatcher) FailureMessage(actual any) string {
	var errorMsg string
	pod, ok := actual.(corev1.Pod)
	if !ok {
		return fmt.Sprintf("expected a pod, got %T", actual)
	}

	if pm.expectedPod.ContainerName != "" {
		errorMsg += fmt.Sprintf("Expected pod %s to have container '%s', but it was not found", client.ObjectKeyFromObject(&pod), pm.expectedPod.ContainerName)
	}
	if pm.expectedPod.Status != "" {
		errorMsg += fmt.Sprintf("Expected pod %s to have status '%s', but it was not found", client.ObjectKeyFromObject(&pod), pm.expectedPod.Status)
	}
	return errorMsg
}

func (pm *podMatcher) NegatedFailureMessage(actual any) string {
	pod := actual.(corev1.Pod)

	var errorMsg string
	if pm.expectedPod.ContainerName != "" {
		var containers strings.Builder
		for _, container := range pod.Spec.Containers {
			containers.WriteString(container.Name + ", ")
		}
		errorMsg += fmt.Sprintf("Expected pod %s to have container '%s', but it found %s", client.ObjectKeyFromObject(&pod), pm.expectedPod.ContainerName, containers.String())
	}
	if pm.expectedPod.Status != "" {
		errorMsg += fmt.Sprintf("Expected pod %s to have status '%s', but it found %s", client.ObjectKeyFromObject(&pod), pm.expectedPod.Status, pod.Status.Phase)
	}
	return errorMsg
}
