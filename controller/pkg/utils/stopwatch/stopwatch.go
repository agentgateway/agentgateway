package stopwatch

import (
	"context"
	"log"
	"time"

	"go.opencensus.io/stats"
	"go.opencensus.io/stats/view"
	"go.opencensus.io/tag"
)

var (
	translationTime      = stats.Float64("io.kgateway/translation_time_sec", "how long the translator takes in seconds", "s")
	translatorNameKey, _ = tag.NewKey("translator_name")
)

func init() {
	// Register views with OpenCensus
	if err := view.Register(
		&view.View{
			Name:        "io.kgateway/translation_time_sec",
			Measure:     translationTime,
			Description: "how long the translator takes in seconds",
			Aggregation: view.Distribution(0.01, 0.05, 0.1, 0.25, 0.5, 1, 5, 10, 60),
			TagKeys:     []tag.Key{translatorNameKey},
		},
	); err != nil {
		log.Fatalf("Failed to register views: %v", err)
	}
}

func NewTranslatorStopWatch(translatorName string) StopWatch {
	return NewStopWatch(translationTime, tag.Upsert(translatorNameKey, translatorName))
}

// StopWatch is a stopwatch that records the duration of an operation and records an opencensus metric for the time between Start and Stop
type StopWatch interface {
	Start()
	Stop(ctx context.Context) time.Duration
}

type stopwatch struct {
	startTime time.Time
	measure   *stats.Float64Measure
	labels    []tag.Mutator
}

// NewStopWatch creates a new StopWatch that records the duration of an operation and records an opencensus metric for the time between Start and Stop
// The metric is recorded with the provided measurement and labels as a tag
func NewStopWatch(measure *stats.Float64Measure, labels ...tag.Mutator) StopWatch {
	return &stopwatch{
		measure: measure,
		labels:  labels,
	}
}

// Start starts the stopwatch
func (s *stopwatch) Start() {
	s.startTime = time.Now()
}

// Stop stops the stopwatch and records the duration of the operation
// Note: Stop() should be called only once per Start() call, otherwise this could lead to double-counting in any
// metrics that rely on this stopwatch and redundant logging.
func (s *stopwatch) Stop(ctx context.Context) time.Duration {
	duration := time.Since(s.startTime)
	tagCtx, _ := tag.New(ctx, s.labels...)
	stats.Record(tagCtx, s.measure.M(duration.Seconds()))
	return duration
}
