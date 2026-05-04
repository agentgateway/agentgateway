package remotecache

import (
	"testing"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestRequestQueueKey(t *testing.T) {
	key := remotehttp.FetchKey("test-key")
	nsName := RequestQueueKey("ns", key)
	require.Equal(t, "ns", nsName.Namespace)
	require.Equal(t, "test-key", nsName.Name)
}

func TestPlanConfigMapSyncUpsertWhenExists(t *testing.T) {
	plan := PlanConfigMapSync(
		[]string{"canonical", "stale-1", "stale-2"},
		"canonical",
		true,
	)
	require.Equal(t, "canonical", plan.UpsertName)
	require.ElementsMatch(t, []string{"stale-1", "stale-2"}, plan.DeleteNames)
}

func TestPlanConfigMapSyncDeleteAllWhenAbsent(t *testing.T) {
	plan := PlanConfigMapSync(
		[]string{"orphan-1", "orphan-2"},
		"",
		false,
	)
	require.Empty(t, plan.UpsertName)
	require.ElementsMatch(t, []string{"orphan-1", "orphan-2"}, plan.DeleteNames)
}

func TestPlanConfigMapSyncEmptyExistingExists(t *testing.T) {
	plan := PlanConfigMapSync(nil, "canonical", true)
	require.Equal(t, "canonical", plan.UpsertName)
	require.Empty(t, plan.DeleteNames)
}

func TestPlanConfigMapSyncEmptyExistingAbsent(t *testing.T) {
	plan := PlanConfigMapSync(nil, "", false)
	require.Empty(t, plan.UpsertName)
	require.Empty(t, plan.DeleteNames)
}
