console.log('KISS Static Server Test - JavaScript loaded successfully');

document.addEventListener('DOMContentLoaded', function() {
    console.log('DOM Content Loaded');
    
    // Simple test to verify JS execution
    const h1 = document.querySelector('h1');
    if (h1) {
        h1.style.transition = 'color 0.3s ease';
        h1.addEventListener('mouseover', () => {
            h1.style.color = '#4CAF50';
        });
        h1.addEventListener('mouseout', () => {
            h1.style.color = '#333';
        });
    }
});