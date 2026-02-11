package gomega

import (
	"time"
)

var (
	// DefaultConsistentlyDuration is the default value for the duration parameter
	// to the async assertion Consistently. Last updated gomega@v1.26.0
	DefaultConsistentlyDuration = time.Millisecond * 100
	// DefaultConsistentlyPollingInterval is the default value for the polling
	// interval parameter to the async assertion Consistently. Last updated gomega@v1.26.0
	DefaultConsistentlyPollingInterval = time.Millisecond * 10
	// DefaultEventuallyTimeout is the default value for the timeout parameter
	// to the async assertion Eventually. Last updated gomega@v1.26.0
	DefaultEventuallyTimeout = time.Second * 1
	// DefaultEventuallyPollingInterval is the default value for the polling
	// interval parameter to the async assertion Eventually. Last updated gomega@v1.26.0
	DefaultEventuallyPollingInterval = time.Millisecond * 10
)
