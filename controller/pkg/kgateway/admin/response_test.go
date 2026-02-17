package admin_test

import (
	"encoding/json"
	"errors"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/admin"
)

func TestSnapshotResponseDataMarshalJSONString(t *testing.T) {
	tests := []struct {
		name         string
		response     admin.SnapshotResponseData
		expectedJSON string
	}{
		{
			name: "successful response can be formatted as json",
			response: admin.SnapshotResponseData{
				Data:  "my data",
				Error: nil,
			},
			expectedJSON: `{"data":"my data","error":""}`,
		},
		{
			name: "errored response can be formatted as json",
			response: admin.SnapshotResponseData{
				Data:  "",
				Error: errors.New("one error"),
			},
			expectedJSON: `{"data":"","error":"one error"}`,
		},
		{
			name: "CR list can be formatted as json",
			response: admin.SnapshotResponseData{
				Data: []corev1.Namespace{
					{
						ObjectMeta: metav1.ObjectMeta{
							Name:      "name",
							Namespace: "namespace",
							ManagedFields: []metav1.ManagedFieldsEntry{{
								Manager: "manager",
							}},
						},
						TypeMeta: metav1.TypeMeta{
							Kind:       "kind",
							APIVersion: "version",
						},
					},
				},
				Error: nil,
			},
			expectedJSON: `{"data":[{"kind":"kind","apiVersion":"version","metadata":{"name":"name","namespace":"namespace","managedFields":[{"manager":"manager"}]},"spec":{},"status":{}}],"error":""}`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			responseStr := tt.response.MarshalJSONString()
			assertJSONEqual(t, tt.expectedJSON, responseStr)
		})
	}
}

func assertJSONEqual(t *testing.T, expected, actual string) {
	t.Helper()

	var expectedObj any
	var actualObj any
	assert.NoError(t, json.Unmarshal([]byte(expected), &expectedObj))
	assert.NoError(t, json.Unmarshal([]byte(actual), &actualObj))
	assert.Equal(t, expectedObj, actualObj)
}
