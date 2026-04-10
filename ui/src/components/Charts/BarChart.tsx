import styled from "@emotion/styled";
import { BarController, BarElement, CategoryScale, Chart, Legend, LinearScale } from "chart.js";
import { useEffect, useRef } from 'react';

Chart.register(BarController, BarElement, CategoryScale, LinearScale, Legend);

const Container = styled.div`
    display: flex;
    flex: 1;
    min-width: 300px;
    flex-direction: column;
    gap: var(--spacing-lg);
    padding: var(--spacing-lg);
    background-color: var(--color-bg-container);
    border: 1px solid var(--color-border-secondary);
    border-radius: var(--border-radius-md);
`;

const ChartCanvas = styled.div<{ height?: string }>`
    position: relative;
    height: ${props => props.height || "350px"};
`;

const ChartTitle = styled.div`
    display: flex;
    align-items: flex-start;
    font-size: 16px;
    font-weight: 600;
    color: var(--color-text-base);
`

interface BarChartDataset { 
    label: string;
    data: number[];
    backgroundColor: string;
}

interface BarChartProps { 
    title: string;
    labels: string[];
    datasets: BarChartDataset[];
    height?: string;
}

export const BarChart = ({ title, labels, datasets, height }: BarChartProps) => { 
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const chartRef = useRef<Chart | null>(null);

    useEffect(() => { 
        if (!canvasRef.current) return;

        // destroy existing instance to avoid canvas reuse errors
        if (chartRef.current) { 
            chartRef.current.destroy();
        }

        chartRef.current = new Chart(canvasRef.current, { 
            type: 'bar',
            data: { 
                labels,
                datasets: datasets.map((ds: BarChartDataset) => ({ 
                    ...ds,
                    backgroundColor: ds.backgroundColor,
                })),
            },
            options: {
                responsive: true,
                maintainAspectRatio: false,
                scales: { 
                    y: { beginAtZero: true },
                },
                plugins: { 
                    legend: { 
                        position: 'top',
                    }
                }
            },
        })
    }, [labels, datasets]);
    
    return (
        <Container>
            <ChartTitle>{title}</ChartTitle>
            <ChartCanvas
                height={height}
            >
                <canvas ref={canvasRef} />
            </ChartCanvas>
        </Container>
    );
}