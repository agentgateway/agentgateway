package helpers

import (
	"fmt"
	"time"

	. "github.com/onsi/gomega"
)

const (
	DefaultTimeout         = time.Second * 60
	DefaultPollingInterval = time.Second * 2
)

var getTimeoutsAsInterfaces = GetDefaultTimingsTransform(DefaultTimeout, DefaultPollingInterval)

func GetTimeouts(timeout ...time.Duration) (currentTimeout, pollingInterval time.Duration) {
	// Convert the timeouts to interface{}s
	interfaceTimeouts := make([]any, len(timeout))
	for i, t := range timeout {
		interfaceTimeouts[i] = t
	}

	timeoutAny, pollingIntervalAny := getTimeoutsAsInterfaces(interfaceTimeouts...)
	currentTimeout = timeoutAny.(time.Duration)
	pollingInterval = pollingIntervalAny.(time.Duration)
	return currentTimeout, pollingInterval
}

// GetDefaultTimingsTransform is used to return the timeout and polling interval values to use with a gomega eventually or consistently call
// It can also be called directly with just 2 arguments if both timeout and polling interval are known and there is no need to default to Gomega values
func GetDefaultTimingsTransform(timeout, polling any, defaults ...any) func(intervals ...any) (any, any) {
	var defaultTimeoutInterval, defaultPollingInterval any
	defaultTimeoutInterval = timeout
	defaultPollingInterval = polling

	// The curl helper doesn't let you set the intervals to 0, so we need to check for that
	if len(defaults) > 0 && defaults[0] != 0 {
		defaultTimeoutInterval = defaults[0]
	}
	if len(defaults) > 1 && defaults[1] != 0 {
		defaultPollingInterval = defaults[1]
	}

	// This function is a closure that will return the timeout and polling intervals
	return func(intervals ...any) (any, any) {
		var timeoutInterval, pollingInterval any
		timeoutInterval = defaultTimeoutInterval
		pollingInterval = defaultPollingInterval

		if len(intervals) > 0 && intervals[0] != 0 {
			durationInterval, err := asDuration(intervals[0])
			Expect(err).NotTo(HaveOccurred())
			if durationInterval != 0 {
				timeoutInterval = durationInterval
			}
		}
		if len(intervals) > 1 && intervals[1] != 0 {
			durationInterval, err := asDuration(intervals[1])
			Expect(err).NotTo(HaveOccurred())
			if durationInterval != 0 {
				pollingInterval = durationInterval
			}
		}

		return timeoutInterval, pollingInterval
	}
}

func asDuration(d any) (time.Duration, error) {
	if duration, ok := d.(time.Duration); ok {
		return duration, nil
	}

	if duration, ok := d.(string); ok {
		parsedDuration, err := time.ParseDuration(duration)
		if err != nil {
			return 0, err
		}
		return parsedDuration, nil
	}

	return 0, fmt.Errorf("could not convert %v to time.Duration", d)
}
